//! SDF 文本渲染模块，模拟 TMP `UNDERLAY_ON` shader variant 的双层效果。
//!
//! 当前生产路径只有 outline SDF：
//! 1. 按字体轮廓动态生成 glyph SDF
//! 2. 在进程内按 `(font_family, char)` 缓存生成结果
//! 3. 依据 live 材质参数重建 face / underlay coverage

use skia_safe::{
    AlphaType, Canvas, Color4f, ColorType, Data, FilterMode, Font, ImageInfo, MipmapMode, Paint,
    Point, Rect, SamplingOptions,
};

use crate::sdf::outline as outline_sdf;

use rayon::prelude::*;
use std::sync::{Arc, OnceLock};

/// 光栅化性能诊断计数器（benchmark 专用，默认零开销 relaxed 累加）。
/// 统计每次渲染调用 rasterize_local_rect_to_device 的字形数 / 总像素数 / shade 调用数。
pub mod bench_counters {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    pub static GLYPHS: AtomicU64 = AtomicU64::new(0);
    pub static PIXELS: AtomicU64 = AtomicU64::new(0);
    pub static SHADE_CALLS: AtomicU64 = AtomicU64::new(0);
    /// pre-pad 的 mapped_rect 面积总和（区分「字形本身大」vs「pad 撑爆」）
    pub static PREPAD_PIXELS: AtomicU64 = AtomicU64::new(0);
    /// pad_px 总和（除以字形数得平均 pad）
    pub static PAD_SUM: AtomicU64 = AtomicU64::new(0);
    /// 最大单字形窗口像素
    pub static MAX_WINDOW: AtomicU64 = AtomicU64::new(0);
    /// 巨型字降分辨率开关：true 时缓冲取满设备窗口分辨率（精确光栅化，不降采样），
    /// 供 A/B 工具量化「精确 vs 降采样」的视觉差。生产恒为 false（启用降采样）。
    pub static DISABLE_DOWNSAMPLE: AtomicBool = AtomicBool::new(false);

    pub fn reset() {
        GLYPHS.store(0, Ordering::Relaxed);
        PIXELS.store(0, Ordering::Relaxed);
        SHADE_CALLS.store(0, Ordering::Relaxed);
        PREPAD_PIXELS.store(0, Ordering::Relaxed);
        PAD_SUM.store(0, Ordering::Relaxed);
        MAX_WINDOW.store(0, Ordering::Relaxed);
    }
    pub fn snapshot() -> (u64, u64, u64, u64, u64, u64) {
        (
            GLYPHS.load(Ordering::Relaxed),
            PIXELS.load(Ordering::Relaxed),
            SHADE_CALLS.load(Ordering::Relaxed),
            PREPAD_PIXELS.load(Ordering::Relaxed),
            PAD_SUM.load(Ordering::Relaxed),
            MAX_WINDOW.load(Ordering::Relaxed),
        )
    }
}

const TMP_SHADER_CLAMP: f32 = 1.0;
const GRADIENT_SCALE: f32 = 6.0;
const FACE_DILATE: f32 = 0.0;
const OUTLINE_WIDTH: f32 = 0.0;
const OUTLINE_SOFTNESS: f32 = 0.0;
const UNDERLAY_SOFTNESS: f32 = 0.0;
const UNDERLAY_OFFSET_X: f32 = 0.0;
const UNDERLAY_OFFSET_Y: f32 = 0.0;
const WEIGHT_NORMAL: f32 = 0.0;
const WEIGHT_BOLD: f32 = 0.75;
const SHARPNESS: f32 = 0.0;
const DEFAULT_RUNTIME_PADDING: f32 = 5.3125;
const DEFAULT_RUNTIME_SCALE_RATIO_C: f32 = 0.6770833;
const DEFAULT_OUTLINE_GRAY_BIAS: f32 = 0.0;
const DEFAULT_RUNTIME_SCREEN_X: f32 = 1920.0;
const DEFAULT_RUNTIME_SCREEN_Y: f32 = 1080.0;
const DEFAULT_RUNTIME_PROJ0_X: f32 = 0.5625;
const DEFAULT_RUNTIME_PROJ0_Y: f32 = 0.0;
const DEFAULT_RUNTIME_PROJ1_X: f32 = 0.0;
const DEFAULT_RUNTIME_PROJ1_Y: f32 = 1.0;
const DEFAULT_RUNTIME_GL_POSITION_W: f32 = 1.0;
const DEFAULT_RUNTIME_SCALE_X: f32 = 1.0;
const DEFAULT_RUNTIME_SCALE_Y: f32 = 1.0;

#[derive(Clone, Copy)]
struct TmpShaderParams {
    uv2_y: f32,
    pixel_scale: f32,
    scale_ratio_a: f32,
    scale_ratio_c: f32,
    face_bias: f32,
    face_scale: f32,
    underlay_bias: f32,
    underlay_scale: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct RuntimeLikeGlyphMeshCarrier {
    pub point_size: f32,
    pub uv2_y: f32,
    pub vertex_alpha_u8: u8,
}

impl RuntimeLikeGlyphMeshCarrier {
    pub fn vertex_alpha(self) -> f32 {
        self.vertex_alpha_u8 as f32 / 255.0
    }
}

#[derive(Clone, Copy)]
struct GlyphSource<'a> {
    glyph: &'a outline_sdf::OutlineSdfGlyph,
}

impl GlyphSource<'_> {
    fn sampling_point_size(self) -> f32 {
        outline_sdf::sampling_point_size()
    }

    fn atlas_padding(self) -> f32 {
        outline_sdf::atlas_padding()
    }

    fn spread(self) -> f32 {
        self.atlas_padding() + 1.0
    }

    fn plane_bearing_x(self) -> f32 {
        self.glyph.plane_bearing_x()
    }

    fn plane_bearing_y(self) -> f32 {
        self.glyph.plane_bearing_y()
    }

    fn plane_width(self) -> f32 {
        self.glyph.plane_width()
    }

    fn plane_height(self) -> f32 {
        self.glyph.plane_height()
    }

    fn sample_width(self) -> f32 {
        self.glyph.width() as f32
    }

    fn sample_height(self) -> f32 {
        self.glyph.height() as f32
    }

    fn sample_gray_or_zero(self, x: f32, y: f32) -> f32 {
        (self.glyph.sample_gray_or_zero(x, y) + outline_gray_bias()).clamp(0.0, 1.0)
    }
}

fn lookup_glyph_source(
    font_family: Option<&str>,
    ch: char,
) -> Option<Arc<outline_sdf::OutlineSdfGlyph>> {
    outline_sdf::lookup_or_generate(font_family, ch)
}

fn env_f32(name: &str, default: f32) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(default)
}

fn env_f32_any(name: &str, default: f32) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .unwrap_or(default)
}

fn runtime_padding() -> f32 {
    static PADDING: OnceLock<f32> = OnceLock::new();
    *PADDING.get_or_init(|| env_f32("SCAPUS_TMP_RUNTIME_PADDING", DEFAULT_RUNTIME_PADDING))
}

fn outline_gray_bias() -> f32 {
    static BIAS: OnceLock<f32> = OnceLock::new();
    *BIAS.get_or_init(|| env_f32_any("SCAPUS_TMP_OUTLINE_GRAY_BIAS", DEFAULT_OUTLINE_GRAY_BIAS))
}

fn runtime_scale_ratio_c() -> f32 {
    static RATIO: OnceLock<f32> = OnceLock::new();
    *RATIO.get_or_init(|| env_f32("SCAPUS_TMP_SCALE_RATIO_C", DEFAULT_RUNTIME_SCALE_RATIO_C))
}

fn runtime_uv2_y_from_point_size(point_size: f32) -> f32 {
    const K: f32 = 1.0 / 20250.0;
    let ps = point_size.abs();
    if !ps.is_finite() || ps <= 0.0 {
        return 1e-8;
    }
    (ps * K).max(1e-8)
}

pub fn runtime_like_mesh_carrier(
    point_size: f32,
    is_bold: bool,
    vertex_alpha_u8: u8,
) -> RuntimeLikeGlyphMeshCarrier {
    let uv2_y_mag = runtime_uv2_y_from_point_size(point_size);
    RuntimeLikeGlyphMeshCarrier {
        point_size,
        uv2_y: if is_bold { -uv2_y_mag } else { uv2_y_mag },
        vertex_alpha_u8,
    }
}

fn runtime_scale_x() -> f32 {
    static SCALE_X: OnceLock<f32> = OnceLock::new();
    *SCALE_X.get_or_init(|| env_f32("SCAPUS_TMP_RUNTIME_SCALE_X", DEFAULT_RUNTIME_SCALE_X))
}

fn runtime_scale_y() -> f32 {
    static SCALE_Y: OnceLock<f32> = OnceLock::new();
    *SCALE_Y.get_or_init(|| env_f32("SCAPUS_TMP_RUNTIME_SCALE_Y", DEFAULT_RUNTIME_SCALE_Y))
}

fn runtime_screen_x() -> f32 {
    static SCREEN_X: OnceLock<f32> = OnceLock::new();
    *SCREEN_X.get_or_init(|| env_f32("SCAPUS_TMP_RUNTIME_SCREEN_X", DEFAULT_RUNTIME_SCREEN_X))
}

fn runtime_screen_y() -> f32 {
    static SCREEN_Y: OnceLock<f32> = OnceLock::new();
    *SCREEN_Y.get_or_init(|| env_f32("SCAPUS_TMP_RUNTIME_SCREEN_Y", DEFAULT_RUNTIME_SCREEN_Y))
}

fn runtime_proj0_x() -> f32 {
    static VALUE: OnceLock<f32> = OnceLock::new();
    *VALUE.get_or_init(|| env_f32("SCAPUS_TMP_RUNTIME_PROJ0_X", DEFAULT_RUNTIME_PROJ0_X))
}

fn runtime_proj0_y() -> f32 {
    static VALUE: OnceLock<f32> = OnceLock::new();
    *VALUE.get_or_init(|| env_f32_any("SCAPUS_TMP_RUNTIME_PROJ0_Y", DEFAULT_RUNTIME_PROJ0_Y))
}

fn runtime_proj1_x() -> f32 {
    static VALUE: OnceLock<f32> = OnceLock::new();
    *VALUE.get_or_init(|| env_f32_any("SCAPUS_TMP_RUNTIME_PROJ1_X", DEFAULT_RUNTIME_PROJ1_X))
}

fn runtime_proj1_y() -> f32 {
    static VALUE: OnceLock<f32> = OnceLock::new();
    *VALUE.get_or_init(|| env_f32("SCAPUS_TMP_RUNTIME_PROJ1_Y", DEFAULT_RUNTIME_PROJ1_Y))
}

fn runtime_gl_position_w() -> f32 {
    static VALUE: OnceLock<f32> = OnceLock::new();
    *VALUE.get_or_init(|| {
        env_f32(
            "SCAPUS_TMP_RUNTIME_GL_POSITION_W",
            DEFAULT_RUNTIME_GL_POSITION_W,
        )
    })
}

fn compute_orthographic_pixel_scale() -> f32 {
    let proj_xy_x =
        (runtime_proj0_x() * runtime_screen_x() + runtime_proj1_x() * runtime_screen_y()).abs()
            * runtime_scale_x().abs().max(1e-6);
    let proj_xy_y =
        (runtime_proj0_y() * runtime_screen_x() + runtime_proj1_y() * runtime_screen_y()).abs()
            * runtime_scale_y().abs().max(1e-6);
    let pixel_size_x = runtime_gl_position_w() / proj_xy_x.max(1e-6);
    let pixel_size_y = runtime_gl_position_w() / proj_xy_y.max(1e-6);
    let pixel_scale = 1.0
        / (pixel_size_x * pixel_size_x + pixel_size_y * pixel_size_y)
            .sqrt()
            .max(1e-6);
    if pixel_scale.is_finite() && pixel_scale > 0.0001 {
        pixel_scale
    } else {
        0.0001
    }
}

fn compute_pixel_scale_from_terms() -> f32 {
    let pixel_scale = compute_orthographic_pixel_scale();
    if pixel_scale.is_finite() && pixel_scale > 0.0001 {
        pixel_scale
    } else {
        0.0001
    }
}

fn compute_shader_scale_from_terms(uv2_y: f32, pixel_scale: f32) -> f32 {
    let shader_scale = uv2_y.abs() * pixel_scale * GRADIENT_SCALE * (SHARPNESS + 1.0);
    if shader_scale.is_finite() && shader_scale > 0.0001 {
        shader_scale
    } else {
        0.0001
    }
}

fn compute_shader_params(
    _canvas: &Canvas,
    carrier: RuntimeLikeGlyphMeshCarrier,
    underlay_dilate: f32,
    _fx_scale_x: f32,
) -> TmpShaderParams {
    let pixel_scale = compute_pixel_scale_from_terms();
    let shader_scale = compute_shader_scale_from_terms(carrier.uv2_y, pixel_scale);
    let ratio_weight_dilate = WEIGHT_NORMAL.max(WEIGHT_BOLD) * 0.25;
    let selected_weight_dilate = if carrier.uv2_y <= 0.0 {
        WEIGHT_BOLD
    } else {
        WEIGHT_NORMAL
    } * 0.25;

    let ratio_face_dilate = FACE_DILATE + ratio_weight_dilate;
    let selected_face_dilate = FACE_DILATE + selected_weight_dilate;
    let face_denom = (OUTLINE_SOFTNESS + OUTLINE_WIDTH + ratio_face_dilate).max(1.0);
    let scale_ratio_a =
        ((GRADIENT_SCALE - TMP_SHADER_CLAMP) / (GRADIENT_SCALE * face_denom)).max(0.0);
    let face_softness = OUTLINE_SOFTNESS * scale_ratio_a;
    let face_scale = shader_scale / (1.0 + face_softness * shader_scale);
    let face_base = 0.5 - selected_face_dilate * scale_ratio_a * 0.5;
    let face_bias = face_base * face_scale - 0.5;

    let scale_ratio_c = runtime_scale_ratio_c();
    let underlay_softness = UNDERLAY_SOFTNESS * scale_ratio_c;
    let underlay_scale = shader_scale / (1.0 + underlay_softness * shader_scale);
    let underlay_bias =
        face_base * underlay_scale - 0.5 - (underlay_dilate * scale_ratio_c) * underlay_scale * 0.5;

    TmpShaderParams {
        uv2_y: carrier.uv2_y,
        pixel_scale,
        scale_ratio_a,
        scale_ratio_c,
        face_bias,
        face_scale,
        underlay_bias,
        underlay_scale,
    }
}

fn glyph_plane_rect(glyph: GlyphSource<'_>, pos: Point, logical_scale: f32) -> Rect {
    let spread = glyph.spread();
    Rect::from_xywh(
        pos.x + (glyph.plane_bearing_x() - spread) * logical_scale,
        pos.y - (glyph.plane_bearing_y() + spread) * logical_scale,
        (glyph.plane_width() + spread * 2.0) * logical_scale,
        (glyph.plane_height() + spread * 2.0) * logical_scale,
    )
}

fn sample_sdf_alpha(glyph: GlyphSource<'_>, local_rect: Rect, local_x: f32, local_y: f32) -> f32 {
    let width = local_rect.width().max(1e-6);
    let height = local_rect.height().max(1e-6);
    let u = (local_x - local_rect.left) / width;
    let v = (local_y - local_rect.top) / height;
    let src_x = u * glyph.sample_width() - 0.5;
    let src_y = v * glyph.sample_height() - 0.5;
    glyph.sample_gray_or_zero(src_x, src_y)
}

fn draw_rgba_bitmap_identity(
    canvas: &Canvas,
    pixels: &[u8],
    width: usize,
    height: usize,
    dest_x: f32,
    dest_y: f32,
    dest_w: f32,
    dest_h: f32,
) -> bool {
    let row_bytes = width * 4;
    let info = ImageInfo::new(
        (width as i32, height as i32),
        ColorType::RGBA8888,
        AlphaType::Premul,
        None,
    );
    let data = Data::new_copy(pixels);
    let image = match skia_safe::images::raster_from_data(&info, data, row_bytes) {
        Some(image) => image,
        None => return false,
    };
    let dst_rect = Rect::from_xywh(dest_x, dest_y, dest_w, dest_h);
    let paint = Paint::default();
    // 1:1 用 Nearest（与历史逐字节一致）；降采样缓冲放大贴回时用 Linear 双线性。
    let upscaling = (dest_w - width as f32).abs() > 0.5 || (dest_h - height as f32).abs() > 0.5;
    let filter = if upscaling {
        FilterMode::Linear
    } else {
        FilterMode::Nearest
    };
    let sampling = SamplingOptions::new(filter, MipmapMode::None);
    canvas.save();
    canvas.reset_matrix();
    canvas.draw_image_rect_with_sampling_options(&image, None, &dst_rect, sampling, &paint);
    canvas.restore();
    true
}

fn device_sample_phase_x() -> f32 {
    static PHASE: OnceLock<f32> = OnceLock::new();
    *PHASE.get_or_init(|| env_f32_any("SCAPUS_TMP_DEVICE_SAMPLE_PHASE_X", 0.0))
}

fn device_sample_phase_y() -> f32 {
    static PHASE: OnceLock<f32> = OnceLock::new();
    *PHASE.get_or_init(|| env_f32_any("SCAPUS_TMP_DEVICE_SAMPLE_PHASE_Y", 0.0))
}

/// 巨型字降采样的目标过采样倍率（缓冲分辨率下限 = N× 源 SDF 分辨率）。默认 4.0：
/// 设备窗口 ≤ N× 源分辨率的字（普通字号）rw=width、scale=1.0，采样式精确退化为原式，逐字节等价。
/// 越小越激进（buffer 越小、越快、边缘越软）。
fn downsample_oversample() -> f32 {
    static O: OnceLock<f32> = OnceLock::new();
    *O.get_or_init(|| {
        std::env::var("SCAPUS_DOWNSAMPLE_OVERSAMPLE")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite() && *v >= 0.5 && *v <= 16.0)
            .unwrap_or(4.0)
    })
}

/// 降采样比例上限：单个缓冲像素最多覆盖几个设备像素。默认 2.0。
/// SDF 本是分辨率无关的——巨型字本该在设备分辨率上 shade 出 ~1px 锐利边；
/// 若只在 oversample×src 缓冲里 shade 再双线性放大 N×，1px 的 AA 边被展宽成 Npx，
/// 抹掉 SDF 对大字最值钱的优势。无此上限时 `<size=400>` 放大 ~3× 实测最大差 107/255、明显发糊。
/// 此上限把缓冲拉回 ≥ 设备窗口/max_scale，将放大倍率（边缘展宽）钳在 max_scale 内，
/// 故最坏退化与字号无关。A/B 实测 seq=2（含 `<size=400>`）max_scale=2.0：总像素 −65%、
/// 最大差 61/255（无 cap 是 107），最坏用例经人眼验图看不出区别。
/// 设为 1.0 等于关闭降采样（缓冲恒取满设备窗口，逐字节精确）；越大越省、边缘越软。
fn downsample_max_scale() -> f32 {
    static S: OnceLock<f32> = OnceLock::new();
    *S.get_or_init(|| {
        std::env::var("SCAPUS_DOWNSAMPLE_MAX_SCALE")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite() && *v >= 1.0 && *v <= 16.0)
            .unwrap_or(2.0)
    })
}

fn production_supersample_grid() -> usize {
    static GRID: OnceLock<usize> = OnceLock::new();
    *GRID.get_or_init(|| {
        std::env::var("SCAPUS_TMP_PROD_SUPERSAMPLE_GRID")
            .ok()
            .and_then(|value| value.trim().parse::<usize>().ok())
            .filter(|value| *value >= 1 && *value <= 8)
            .unwrap_or(1)
    })
}

/// 专用光栅化线程池（**不复用 rayon 全局池**）。
///
/// 这里做的是 CPU 软件光栅化：对窗口内每个设备像素反变换到 local 空间、采样距离场、
/// 超采样累加，把字形轮廓变成像素位图。是纯 CPU 算术，区别于 GPU 硬件光栅化管线。
///
/// 生产渲染由外层单 worker 线程串行驱动，任意时刻至多一个渲染在跑，
/// 故此池进程内全局共享。线程数由 `SCAPUS_RASTER_THREADS` 控制（默认 2，匹配 AS 机
/// 2 物理核）。设为 1 时 `par_chunks_mut` 退化为串行执行，无需单独代码路径。
fn raster_pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        let threads = std::env::var("SCAPUS_RASTER_THREADS")
            .ok()
            .and_then(|value| value.trim().parse::<usize>().ok())
            .filter(|n| (1..=16).contains(n))
            .unwrap_or(2);
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("raster-{i}"))
            .build()
            .expect("光栅化线程池创建失败")
    })
}

fn rasterize_local_rect_to_device<F>(
    canvas: &Canvas,
    local_rect: Rect,
    pad_px: i32,
    src_w: f32,
    src_h: f32,
    shade_pixel: F,
) -> bool
where
    F: Fn(f32, f32) -> Option<[f32; 4]> + Sync,
{
    let supersample_grid = production_supersample_grid();

    let matrix = canvas.local_to_device_as_3x3();
    let inv = match matrix.invert() {
        Some(inv) => inv,
        None => return false,
    };
    let (mapped_rect, _) = matrix.map_rect(local_rect);
    let mut left = mapped_rect.left.floor() as i32 - pad_px;
    let mut top = mapped_rect.top.floor() as i32 - pad_px;
    let mut right = mapped_rect.right.ceil() as i32 + pad_px;
    let mut bottom = mapped_rect.bottom.ceil() as i32 + pad_px;
    if right <= left || bottom <= top {
        return false;
    }

    // 将设备空间光栅化窗口裁剪到画布边界：画布外的像素本就不可见，
    // 但富文本 <size>/<scale> 放大或离屏坐标会让 mapped_rect 膨胀到上千万像素，
    // 逐像素 SDF 采样吃掉数十秒 CPU + 上 GB 内存。裁剪后离屏字形窗口为空直接跳过，
    // 画布内超大字形也被限制在 surface 尺寸内。纯性能裁剪，不改变可见结果。
    let device = canvas.base_layer_size();
    left = left.max(0);
    top = top.max(0);
    right = right.min(device.width);
    bottom = bottom.min(device.height);
    if right <= left || bottom <= top {
        return false;
    }

    let width = (right - left) as usize;
    let height = (bottom - top) as usize;

    // 巨型字降分辨率：源 SDF 仅 ~80px，富文本 <size>/<scale> 把它放大到上千像素是纯过采样。
    // 缓冲分辨率取 min(设备窗口, max(OVERSAMPLE×源SDF分辨率, 设备窗口/MAX_SCALE))，
    // 再双线性放大贴回设备矩形。
    //
    // 两道下限的分工：
    // - OVERSAMPLE×src：中小字的下限，保证缩到 ≥OVERSAMPLE× 源分辨率，量化掉中间带过采样。
    // - 设备窗口/MAX_SCALE：巨型字的下限，把放大倍率（缓冲→设备）钳在 MAX_SCALE 内，
    //   防止在小缓冲里 shade 出 AA 边再放大 N× 抹糊（`<size=400>` 巨型字的退化源）。
    //   字越大这条越占主导，缓冲随窗口按比例增长，边缘展宽恒 ≤ MAX_SCALE。
    // 设备窗口 ≤ OVERSAMPLE×src（普通字）时 rw=width,rh=height,scale=1.0，
    // 采样公式精确退化为原式，逐字节等价、零影响。
    let oversample = downsample_oversample();
    let max_scale = downsample_max_scale();
    let (rw, rh) =
        if bench_counters::DISABLE_DOWNSAMPLE.load(std::sync::atomic::Ordering::Relaxed) {
            (width, height)
        } else {
            let floor_w = (oversample * src_w.max(1.0)).max(width as f32 / max_scale);
            let floor_h = (oversample * src_h.max(1.0)).max(height as f32 / max_scale);
            let target_w = floor_w.ceil() as usize;
            let target_h = floor_h.ceil() as usize;
            (width.min(target_w).max(1), height.min(target_h).max(1))
        };
    // 每个缓冲像素覆盖的设备像素数（连续）。rw=width 时恰为 1.0（整数同值相除位精确）。
    let scale_x = width as f32 / rw as f32;
    let scale_y = height as f32 / rh as f32;

    let mut pixels = vec![0u8; rw * rh * 4];
    let phase_x = device_sample_phase_x();
    let phase_y = device_sample_phase_y();

    {
        use bench_counters::*;
        use std::sync::atomic::Ordering;
        GLYPHS.fetch_add(1, Ordering::Relaxed);
        let win = (rw * rh) as u64; // 实际光栅化像素（降采样后）
        PIXELS.fetch_add(win, Ordering::Relaxed);
        SHADE_CALLS.fetch_add(
            (rw * rh * supersample_grid * supersample_grid) as u64,
            Ordering::Relaxed,
        );
        // pre-pad 面积：mapped_rect 本身的设备像素数（不含 pad_px 膨胀）
        let prepad_w = (mapped_rect.right.ceil() - mapped_rect.left.floor()).max(0.0) as u64;
        let prepad_h = (mapped_rect.bottom.ceil() - mapped_rect.top.floor()).max(0.0) as u64;
        PREPAD_PIXELS.fetch_add(prepad_w * prepad_h, Ordering::Relaxed);
        PAD_SUM.fetch_add(pad_px.max(0) as u64, Ordering::Relaxed);
        MAX_WINDOW.fetch_max(win, Ordering::Relaxed);
    }

    // ss_step 含 scale：scale=1 时 = 1/grid（与原式一致）；scale>1 时每个缓冲像素
    // 覆盖 scale 宽的设备区域，采样点落在该区域内。X/Y 现在可不同比例。
    let ss_step_x = scale_x / supersample_grid as f32;
    let ss_step_y = scale_y / supersample_grid as f32;
    let inv_ss = 1.0 / (supersample_grid * supersample_grid) as f32;

    // 每行像素相互独立，按行切分并行光栅化。每像素计算只读共享状态
    // （inv 矩阵 / shade 闭包 / glyph SDF），写入本行专属切片，无数据竞争。
    // 逐像素数学与串行版逐字节一致，输出不随线程数变化。
    let painted = raster_pool().install(|| {
        pixels
            .par_chunks_mut(rw * 4)
            .enumerate()
            .map(|(ry, row)| {
                let mut row_painted = false;
                for rx in 0..rw {
                    let mut accum = [0.0_f32; 4];
                    let mut hit = false;
                    for sy in 0..supersample_grid {
                        for sx in 0..supersample_grid {
                            let device_x = left as f32
                                + rx as f32 * scale_x
                                + (sx as f32 + 0.5) * ss_step_x
                                + phase_x;
                            let device_y = top as f32
                                + ry as f32 * scale_y
                                + (sy as f32 + 0.5) * ss_step_y
                                + phase_y;
                            let local = inv.map_point((device_x, device_y));
                            let Some(sample) = shade_pixel(local.x, local.y) else {
                                continue;
                            };
                            for i in 0..4 {
                                accum[i] += sample[i];
                            }
                            hit = true;
                        }
                    }
                    if !hit {
                        continue;
                    }
                    let idx = rx * 4;
                    row[idx] = (accum[0] * inv_ss * 255.0).round().clamp(0.0, 255.0) as u8;
                    row[idx + 1] = (accum[1] * inv_ss * 255.0).round().clamp(0.0, 255.0) as u8;
                    row[idx + 2] = (accum[2] * inv_ss * 255.0).round().clamp(0.0, 255.0) as u8;
                    row[idx + 3] = (accum[3] * inv_ss * 255.0).round().clamp(0.0, 255.0) as u8;
                    row_painted = true;
                }
                row_painted
            })
            .reduce(|| false, |a, b| a || b)
    });

    // 缓冲分辨率 rw×rh 放大贴回设备矩形 width×height。rw=width 时 1:1 → Nearest 逐字节等价。
    painted
        && draw_rgba_bitmap_identity(
            canvas,
            &pixels,
            rw,
            rh,
            left as f32,
            top as f32,
            width as f32,
            height as f32,
        )
}

/// Underlay 参数。
pub struct SdfOutlineParams {
    pub outline_r: f32,
    pub outline_g: f32,
    pub outline_b: f32,
    pub outline_a: f32,
    pub outline_size: f32,
    pub font_size: f32,
}

/// 兼容旧调用名；正式路径只走轮廓动态 SDF。
pub fn render_char_face_from_atlas(
    canvas: &Canvas,
    ch: &str,
    pos: Point,
    font: &Font,
    font_family: Option<&str>,
    carrier: RuntimeLikeGlyphMeshCarrier,
    fx_scale_x: f32,
    face_color: Color4f,
) -> bool {
    render_char_face_from_outline(
        canvas,
        ch,
        pos,
        font,
        font_family,
        carrier,
        fx_scale_x,
        face_color,
    )
}

fn render_char_face_from_outline(
    canvas: &Canvas,
    ch: &str,
    pos: Point,
    font: &Font,
    font_family: Option<&str>,
    carrier: RuntimeLikeGlyphMeshCarrier,
    fx_scale_x: f32,
    face_color: Color4f,
) -> bool {
    let mut chars = ch.chars();
    let ch = match (chars.next(), chars.next()) {
        (Some(ch), None) => ch,
        _ => return false,
    };
    if ch.is_whitespace() {
        return true;
    }
    let glyph = match lookup_glyph_source(font_family, ch) {
        Some(glyph) => glyph,
        None => return false,
    };
    let glyph = GlyphSource {
        glyph: glyph.as_ref(),
    };

    let point_size = glyph.sampling_point_size();
    if point_size <= 0.0 {
        return false;
    }

    let logical_scale = font.size() / point_size;
    if logical_scale <= 0.0 {
        return false;
    }

    let local_rect = glyph_plane_rect(glyph, pos, logical_scale);
    let shader = compute_shader_params(canvas, carrier, 0.0, fx_scale_x);
    let _ = (shader.uv2_y, shader.pixel_scale);
    rasterize_local_rect_to_device(
        canvas,
        local_rect,
        1,
        glyph.sample_width(),
        glyph.sample_height(),
        |local_x, local_y| {
            let sdf_a = sample_sdf_alpha(glyph, local_rect, local_x, local_y);
            let face_t = (sdf_a * shader.face_scale - shader.face_bias).clamp(0.0, 1.0);
            if face_t <= 0.0 {
                return None;
            }
            let face_alpha = face_color.a * face_t;
            let vertex_alpha = carrier.vertex_alpha();
            Some([
                face_color.r * face_alpha * vertex_alpha,
                face_color.g * face_alpha * vertex_alpha,
                face_color.b * face_alpha * vertex_alpha,
                face_alpha * vertex_alpha,
            ])
        },
    )
}

/// 兼容旧调用名；正式路径只走轮廓动态 SDF。
pub fn render_char_sdf_from_atlas(
    canvas: &Canvas,
    ch: &str,
    pos: Point,
    font: &Font,
    font_family: Option<&str>,
    carrier: RuntimeLikeGlyphMeshCarrier,
    fx_scale_x: f32,
    face_color: Color4f,
    params: &SdfOutlineParams,
) -> bool {
    render_char_sdf_from_outline(
        canvas,
        ch,
        pos,
        font,
        font_family,
        carrier,
        fx_scale_x,
        face_color,
        params,
    )
}

fn render_char_sdf_from_outline(
    canvas: &Canvas,
    ch: &str,
    pos: Point,
    font: &Font,
    font_family: Option<&str>,
    carrier: RuntimeLikeGlyphMeshCarrier,
    fx_scale_x: f32,
    face_color: Color4f,
    params: &SdfOutlineParams,
) -> bool {
    let mut chars = ch.chars();
    let ch = match (chars.next(), chars.next()) {
        (Some(ch), None) => ch,
        _ => return false,
    };
    if ch.is_whitespace() {
        return true;
    }
    let glyph = match lookup_glyph_source(font_family, ch) {
        Some(glyph) => glyph,
        None => return false,
    };
    let glyph = GlyphSource {
        glyph: glyph.as_ref(),
    };

    let point_size = glyph.sampling_point_size();
    if point_size <= 0.0 {
        return false;
    }

    let logical_scale = font.size() / point_size;
    if logical_scale <= 0.0 {
        return false;
    }

    let local_rect = glyph_plane_rect(glyph, pos, logical_scale);
    let shader = compute_shader_params(canvas, carrier, params.outline_size.max(0.0), fx_scale_x);
    let _ = (shader.uv2_y, shader.pixel_scale);
    let atlas_padding = glyph.atlas_padding().max(0.0).max(runtime_padding());
    let underlay_screen_dilate =
        params.outline_size.max(0.0) * shader.scale_ratio_c * shader.underlay_scale * 0.5;
    let underlay_offset_screen_pad = UNDERLAY_OFFSET_X.abs().max(UNDERLAY_OFFSET_Y.abs())
        * shader.scale_ratio_c
        * shader.underlay_scale
        * 0.5;
    let face_soft_pad = OUTLINE_SOFTNESS * shader.scale_ratio_a * shader.face_scale;
    let underlay_soft_pad = UNDERLAY_SOFTNESS * shader.scale_ratio_c * shader.underlay_scale;
    let pad_px = (atlas_padding
        + underlay_screen_dilate
        + underlay_offset_screen_pad
        + face_soft_pad
        + underlay_soft_pad)
        .ceil()
        .max(1.0) as i32
        + 2;

    let u_a = params.outline_a;
    let u_r = params.outline_r * u_a;
    let u_g = params.outline_g * u_a;
    let u_b = params.outline_b * u_a;
    let f_a = face_color.a;
    let f_r = face_color.r * f_a;
    let f_g = face_color.g * f_a;
    let f_b = face_color.b * f_a;

    rasterize_local_rect_to_device(
        canvas,
        local_rect,
        pad_px,
        glyph.sample_width(),
        glyph.sample_height(),
        |local_x, local_y| {
            // underlay 与 face 共用同一次双线性采样：二者唯一差异是 underlay 的 uv delta，
            // 而 UNDERLAY_OFFSET_X/Y 恒为 0 → delta 为 -0.0 → `u + (-0.0) == u`（IEEE-754
            // 逐位成立）。故两次 gather 取逐位相同的坐标与值，合并为一次，砍掉每像素一次
            // 冗余双线性采样（gather 是热路径瓶颈）。
            let sdf = sample_sdf_alpha(glyph, local_rect, local_x, local_y);
            // SDF spread band cutoff: multiply underlay alpha by a linear ramp
            // sdf ≤ 0.08 → zero underlay. Beyond 0.08 the underlay is unmodified.
            // 0.08 ≈ (spread-1)/spread where spread=6. Eliminates the faint
            // outline-coloured halo that fills the glyph rect at small sizes.
            let underlay_t = (sdf * shader.underlay_scale - shader.underlay_bias).clamp(0.0, 1.0)
                * (sdf * 12.5).clamp(0.0, 1.0);
            let face_t = (sdf * shader.face_scale - shader.face_bias).clamp(0.0, 1.0);
            if underlay_t <= 0.0 && face_t <= 0.0 {
                return None;
            }

            let one_minus_face_a = 1.0 - f_a * face_t;
            let out_r = f_r * face_t + u_r * underlay_t * one_minus_face_a;
            let out_g = f_g * face_t + u_g * underlay_t * one_minus_face_a;
            let out_b = f_b * face_t + u_b * underlay_t * one_minus_face_a;
            let out_a = f_a * face_t + u_a * underlay_t * one_minus_face_a;
            let vertex_alpha = carrier.vertex_alpha();
            Some([
                out_r * vertex_alpha,
                out_g * vertex_alpha,
                out_b * vertex_alpha,
                out_a * vertex_alpha,
            ])
        },
    )
}

/// 统一入口。
pub fn render_char_sdf(
    canvas: &Canvas,
    ch: &str,
    pos: Point,
    font: &Font,
    font_family: Option<&str>,
    carrier: RuntimeLikeGlyphMeshCarrier,
    fx_scale_x: f32,
    face_color: Color4f,
    params: &SdfOutlineParams,
) {
    if render_char_sdf_from_outline(
        canvas,
        ch,
        pos,
        font,
        font_family,
        carrier,
        fx_scale_x,
        face_color,
        params,
    ) {
        return;
    }

    tracing::warn!(
        text = ch,
        font_family = font_family.unwrap_or("<none>"),
        "outline SDF glyph generation failed; falling back to plain text draw"
    );
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color4f(
        Color4f::new(
            face_color.r,
            face_color.g,
            face_color.b,
            face_color.a * carrier.vertex_alpha(),
        ),
        None,
    );
    canvas.draw_str(ch, pos, font, &paint);
}

#[cfg(test)]
mod tests {
    use super::{
        compute_orthographic_pixel_scale, compute_shader_scale_from_terms,
        runtime_like_mesh_carrier, runtime_scale_ratio_c, runtime_uv2_y_from_point_size,
    };

    #[test]
    fn runtime_uv2_y_from_point_size_matches_runtime_samples() {
        const K: f32 = 1.0 / 20250.0;
        for ps in [8.0, 10.0, 18.0, 24.0, 48.0, 72.0, 96.0] {
            let uv2_y = runtime_uv2_y_from_point_size(ps);
            let expected = ps * K;
            assert!(
                (uv2_y - expected).abs() < 1e-8,
                "point_size={ps} uv2_y={uv2_y} expected={expected}"
            );
        }
    }

    #[test]
    fn runtime_like_mesh_carrier_marks_bold_with_negative_uv2() {
        let normal = runtime_like_mesh_carrier(48.0, false, 255);
        let bold = runtime_like_mesh_carrier(48.0, true, 255);
        assert!(normal.uv2_y > 0.0);
        assert!(bold.uv2_y < 0.0);
    }

    #[test]
    fn runtime_scale_ratio_c_matches_live_default() {
        assert!((runtime_scale_ratio_c() - 0.6770833).abs() < 1e-6);
    }

    #[test]
    fn compute_shader_scale_from_terms_reaches_runtime_target_domain() {
        let shader_scale = compute_shader_scale_from_terms(
            runtime_uv2_y_from_point_size(96.0),
            compute_orthographic_pixel_scale(),
        );
        assert!((shader_scale - 21.7215).abs() < 1e-4);
    }
}
