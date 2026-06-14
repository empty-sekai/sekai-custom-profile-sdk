//! 基于 FreeType `NO_HINTING` 轮廓的动态 SDF glyph 生成器。

mod edt;
mod geometry;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use freetype::{face::LoadFlag, Library, RenderMode};
use ttf_parser::Face as TtfFace;

use self::geometry::{
    dist_to_cubic, dist_to_line, dist_to_quad, extract_segments, winding_number, Segment, Vec2,
};

const TMP_POINT_SIZE: f32 = 75.0;
const TMP_ATLAS_PADDING: usize = 5;
const TMP_SPREAD: f32 = 6.0;

const FONT_FILE_MAP: [(&str, &[&str]); 8] = [
    (
        "FZLanTingHei-DB-GBK",
        &["FOT-RodinNTLGPro-DB.ttf", "FOT-RodinNTLGPro-DB.otf"],
    ),
    (
        "FOT-RodinNTLGPro-DB",
        &["FOT-RodinNTLGPro-DB.ttf", "FOT-RodinNTLGPro-DB.otf"],
    ),
    ("FZZhengHei-EB-GBK", &["FOT-SkipProN-B.otf"]),
    ("FOT-SkipProN-B", &["FOT-SkipProN-B.otf"]),
    ("FZShaoEr-M11-JF", &["FOT-PopHappinessStd-EB.otf"]),
    ("FOT-PopHappinessStd-EB", &["FOT-PopHappinessStd-EB.otf"]),
    ("FOT-Yuruka Std UB", &["FOT-YurukaStd-UB.otf"]),
    ("FOT-YurukaStd-UB", &["FOT-YurukaStd-UB.otf"]),
];

/// 生成好的单 glyph SDF。
#[derive(Clone)]
pub struct OutlineSdfGlyph {
    width: usize,
    height: usize,
    bearing_x: f32,
    bearing_y: f32,
    plane_bearing_x: f32,
    plane_bearing_y: f32,
    plane_width: f32,
    plane_height: f32,
    plane_advance_x: f32,
    pixels: Vec<u8>,
}

impl OutlineSdfGlyph {
    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
    pub fn bearing_x(&self) -> f32 {
        self.bearing_x
    }
    pub fn bearing_y(&self) -> f32 {
        self.bearing_y
    }
    pub fn plane_bearing_x(&self) -> f32 {
        self.plane_bearing_x
    }
    pub fn plane_bearing_y(&self) -> f32 {
        self.plane_bearing_y
    }
    pub fn plane_width(&self) -> f32 {
        self.plane_width
    }
    pub fn plane_height(&self) -> f32 {
        self.plane_height
    }
    /// SDF rect X 方向 padding（ceil/floor 舍入导致的非对称偏移）。
    /// TMP 的 unsheared center_x 包含此项。
    pub fn sdf_pad_x(&self) -> f32 {
        let rect_left = self.plane_bearing_x.floor();
        let rect_right = (self.plane_bearing_x + self.plane_width).ceil();
        ((rect_right - rect_left) - self.plane_width) / 2.0
    }
    pub fn plane_advance_x(&self) -> f32 {
        self.plane_advance_x
    }

    pub fn sample_gray(&self, x: f32, y: f32) -> f32 {
        let max_x = self.width.saturating_sub(1) as f32;
        let max_y = self.height.saturating_sub(1) as f32;
        let x = x.clamp(0.0, max_x);
        let y = y.clamp(0.0, max_y);

        let x0 = x.floor() as usize;
        let y0 = y.floor() as usize;
        let x1 = (x0 + 1).min(self.width.saturating_sub(1));
        let y1 = (y0 + 1).min(self.height.saturating_sub(1));
        let tx = x - x0 as f32;
        let ty = y - y0 as f32;

        let v00 = self.pixel_gray(x0, y0);
        let v10 = self.pixel_gray(x1, y0);
        let v01 = self.pixel_gray(x0, y1);
        let v11 = self.pixel_gray(x1, y1);
        let top = v00 + (v10 - v00) * tx;
        let bottom = v01 + (v11 - v01) * tx;
        top + (bottom - top) * ty
    }

    pub fn sample_gray_or_zero(&self, x: f32, y: f32) -> f32 {
        if x < 0.0 || y < 0.0 || x > self.width as f32 - 1.0 || y > self.height as f32 - 1.0 {
            return 0.0;
        }
        self.sample_gray(x, y)
    }

    fn pixel_gray(&self, x: usize, y: usize) -> f32 {
        self.pixels[y * self.width + x] as f32 / 255.0
    }
}

fn font_path_cache() -> &'static Mutex<HashMap<String, Option<PathBuf>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 内存注册字体表（family → 字体字节）。查找优先于文件系统路径，
/// 供无文件系统环境（wasm）或想完全控制字体来源的宿主使用。
fn font_registry() -> &'static Mutex<HashMap<String, Arc<Vec<u8>>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<Vec<u8>>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 注册内存字体。同 family 重复注册时覆盖旧字节，并失效相关 glyph 缓存。
pub fn register_font_bytes(family: &str, bytes: Vec<u8>) {
    if let Ok(mut registry) = font_registry().lock() {
        registry.insert(family.to_string(), Arc::new(bytes));
    }
    if let Ok(mut cache) = glyph_cache().lock() {
        cache.retain(|(cached_family, _), _| cached_family != family);
    }
    if let Ok(mut cache) = glyph_id_cache().lock() {
        cache.retain(|(cached_family, _), _| cached_family != family);
    }
}

/// 解析 family 的字体字节：注册表优先，其次文件系统。
pub(crate) fn resolve_font_bytes(family: &str) -> Option<Arc<Vec<u8>>> {
    if let Some(bytes) = font_registry()
        .lock()
        .ok()
        .and_then(|registry| registry.get(family).cloned())
    {
        return Some(bytes);
    }
    let path = resolve_font_path(family)?;
    load_font_bytes(&path)
}

/// FreeType 内存 face 的字节载体（Arc 共享，避免每次建 face 复制字体）。
struct FontData(Arc<Vec<u8>>);

impl std::borrow::Borrow<[u8]> for FontData {
    fn borrow(&self) -> &[u8] {
        self.0.as_slice()
    }
}

fn glyph_cache() -> &'static Mutex<HashMap<(String, char), Arc<OutlineSdfGlyph>>> {
    static CACHE: OnceLock<Mutex<HashMap<(String, char), Arc<OutlineSdfGlyph>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn font_bytes_cache() -> &'static Mutex<HashMap<PathBuf, Option<Arc<Vec<u8>>>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Option<Arc<Vec<u8>>>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn glyph_id_cache() -> &'static Mutex<HashMap<(String, char), Option<u32>>> {
    static CACHE: OnceLock<Mutex<HashMap<(String, char), Option<u32>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 按 family name 查找字体文件。
pub fn resolve_font_path(family: &str) -> Option<PathBuf> {
    if let Some(cached) = font_path_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(family).cloned())
    {
        return cached;
    }

    let file_names = FONT_FILE_MAP
        .iter()
        .find_map(|(key, files)| (*key == family).then_some(*files))?;
    let mut candidates = Vec::new();
    for file_name in file_names {
        if let Ok(env_dir) = std::env::var("SCAPUS_FONT_DIR") {
            candidates.push(PathBuf::from(&env_dir).join(file_name));
        }
        candidates.push(PathBuf::from("/usr/share/fonts/custom").join(file_name));
        candidates.push(PathBuf::from("assets/fonts").join(file_name));
    }

    let found = candidates.into_iter().find(|path| path.exists());
    if let Ok(mut cache) = font_path_cache().lock() {
        cache.insert(family.to_string(), found.clone());
    }
    found
}

fn load_font_bytes(font_path: &Path) -> Option<Arc<Vec<u8>>> {
    if let Some(cached) = font_bytes_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(font_path).cloned())
    {
        return cached;
    }
    let bytes = fs::read(font_path).ok().map(Arc::new);
    if let Ok(mut cache) = font_bytes_cache().lock() {
        cache.insert(font_path.to_path_buf(), bytes.clone());
    }
    bytes
}

pub(crate) fn load_font_bytes_for_family(family: &str) -> Option<Arc<Vec<u8>>> {
    resolve_font_bytes(family)
}

fn resolve_glyph_id(family: &str, font_bytes: &Arc<Vec<u8>>, ch: char) -> Option<u32> {
    let key = (family.to_string(), ch);
    if let Some(cached) = glyph_id_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(&key).cloned())
    {
        return cached;
    }
    let resolved = TtfFace::parse(font_bytes.as_slice(), 0)
        .ok()
        .and_then(|face| face.glyph_index(ch).map(|gid| gid.0 as u32));
    if let Ok(mut cache) = glyph_id_cache().lock() {
        cache.insert(key, resolved);
    }
    resolved
}

/// EDT 加速开关 + 超采样因子（运行时配置，默认关闭走解析法）。
///
/// `SCAPUS_SDF_EDT` 未设/为 0 → 解析法（亚像素精确，现有生产行为）。
/// 设为 1-4 → EDT 法，值即超采样因子（实测 2x 为精度/性能甜点，
/// MAE≈1.9%，误差 90% 落在抗锯齿边缘带，加速 5-8x）。
fn edt_supersample() -> Option<usize> {
    static CFG: OnceLock<Option<usize>> = OnceLock::new();
    *CFG.get_or_init(|| {
        let raw = std::env::var("SCAPUS_SDF_EDT").ok()?;
        let ss: usize = raw.trim().parse().ok()?;
        (1..=4).contains(&ss).then_some(ss)
    })
}

/// 查询或生成一个 glyph 的 SDF。
pub fn lookup_or_generate(font_family: Option<&str>, ch: char) -> Option<Arc<OutlineSdfGlyph>> {
    let family = font_family?;
    let key = (family.to_string(), ch);
    if let Some(cached) = glyph_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(&key).cloned())
    {
        return Some(cached);
    }

    let bytes = resolve_font_bytes(family)?;
    let glyph = match edt_supersample() {
        Some(ss) => generate_outline_sdf_edt(family, &bytes, ch, ss)
            .or_else(|_| generate_outline_sdf(family, &bytes, ch)) // EDT 失败回退解析法
            .ok()?,
        None => generate_outline_sdf(family, &bytes, ch).ok()?,
    };
    let glyph = Arc::new(glyph);
    if let Ok(mut cache) = glyph_cache().lock() {
        cache.insert(key, glyph.clone());
    }
    Some(glyph)
}

/// 对比工具专用：生成同一 glyph 的解析法 vs EDT 版 SDF，返回两者 + 耗时。
///
/// 不走缓存，每次都重新生成以测量真实性能。`supersample` 为 EDT 超采样因子。
#[cfg(feature = "dev")]
pub fn benchmark_methods(
    font_family: &str,
    ch: char,
    supersample: usize,
) -> Option<(Arc<OutlineSdfGlyph>, std::time::Duration, Arc<OutlineSdfGlyph>, std::time::Duration)> {
    let bytes = resolve_font_bytes(font_family)?;

    let t0 = std::time::Instant::now();
    let analytic = Arc::new(generate_outline_sdf(font_family, &bytes, ch).ok()?);
    let analytic_dur = t0.elapsed();

    let t1 = std::time::Instant::now();
    let edt = Arc::new(generate_outline_sdf_edt(font_family, &bytes, ch, supersample).ok()?);
    let edt_dur = t1.elapsed();

    Some((analytic, analytic_dur, edt, edt_dur))
}

fn generate_outline_sdf(
    family: &str,
    font_bytes: &Arc<Vec<u8>>,
    ch: char,
) -> Result<OutlineSdfGlyph, String> {
    let library = Library::init().map_err(|err| format!("初始化 FreeType 失败: {err:?}"))?;
    let face = library
        .new_memory_face2(FontData(Arc::clone(font_bytes)), 0)
        .map_err(|err| format!("加载字体失败: {err:?}"))?;
    face.set_char_size((TMP_POINT_SIZE as isize) * 64, 0, 72, 72)
        .map_err(|err| format!("设置点阵大小失败: {err:?}"))?;

    let glyph_id = resolve_glyph_id(family, font_bytes, ch)
        .ok_or_else(|| format!("无法从字体 cmap 解析 glyph id: {ch}"))?;
    face.load_glyph(glyph_id, LoadFlag::NO_BITMAP | LoadFlag::NO_HINTING)
        .map_err(|err| format!("按 glyph id 加载字符失败 (gid={glyph_id}): {err:?}"))?;

    let glyph = face.glyph();
    let outline = &glyph.raw().outline;
    if outline.n_contours <= 0 || outline.n_points <= 0 {
        return Err(format!("字符无轮廓: {ch}"));
    }

    let contours = unsafe { extract_segments(outline) };
    if contours.is_empty() {
        return Err(format!("字符轮廓为空: {ch}"));
    }

    let metrics = glyph.metrics();
    let bear_x = metrics.horiBearingX as f32 / 64.0;
    let bear_y = metrics.horiBearingY as f32 / 64.0;
    let met_w = metrics.width as f32 / 64.0;
    let met_h = metrics.height as f32 / 64.0;

    let rect_left_px = bear_x.floor();
    let rect_top_px = bear_y.ceil();
    let rect_right_px = (bear_x + met_w).ceil();
    let rect_bottom_px = (bear_y - met_h).floor();
    let spread_px = TMP_SPREAD.ceil();
    let sample_left_px = rect_left_px - spread_px;
    let sample_top_px = rect_top_px + spread_px;
    let sample_right_px = rect_right_px + spread_px;
    let sample_bottom_px = rect_bottom_px - spread_px;

    let width = (sample_right_px - sample_left_px).max(1.0) as usize;
    let height = (sample_top_px - sample_bottom_px).max(1.0) as usize;
    let bearing_x = sample_left_px;
    let bearing_y = sample_top_px;
    let rect_left_26_6 = sample_left_px * 64.0;
    let rect_top_26_6 = sample_top_px * 64.0;

    let mut pixels = vec![0u8; width * height];
    for py in 0..height {
        for px in 0..width {
            let point = Vec2::new(
                rect_left_26_6 + (px as f32 + 0.5) * 64.0,
                rect_top_26_6 - (py as f32 + 0.5) * 64.0,
            );
            let mut min_dist = f32::INFINITY;
            for contour in &contours {
                for seg in contour {
                    min_dist = min_dist.min(match *seg {
                        Segment::Line(seg) => dist_to_line(point, seg),
                        Segment::Quad(seg) => dist_to_quad(point, seg),
                        Segment::Cubic(seg) => dist_to_cubic(point, seg),
                    });
                }
            }
            let sign = if winding_number(point, &contours) != 0 {
                -1.0
            } else {
                1.0
            };
            let dist_px = min_dist / 64.0;
            let gray = (0.5 - sign * dist_px / (2.0 * TMP_SPREAD)).clamp(0.0, 1.0);
            pixels[py * width + px] = (gray * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    Ok(OutlineSdfGlyph {
        width,
        height,
        bearing_x,
        bearing_y,
        plane_bearing_x: bear_x,
        plane_bearing_y: bear_y,
        plane_width: met_w.max(1.0 / 64.0),
        plane_height: met_h.max(1.0 / 64.0),
        plane_advance_x: (metrics.horiAdvance as f32) / 64.0,
        pixels,
    })
}

/// EDT 版 SDF 生成（基于 FreeType 光栅化 + 欧几里得距离变换）。
///
/// 与解析法对齐到相同的 width/height/bearing 网格，仅像素填充算法不同。
/// `supersample` 为超采样因子（1=无超采样，2/4=提升精度但增加计算量）。
fn generate_outline_sdf_edt(
    family: &str,
    font_bytes: &Arc<Vec<u8>>,
    ch: char,
    supersample: usize,
) -> Result<OutlineSdfGlyph, String> {
    let library = Library::init().map_err(|err| format!("初始化 FreeType 失败: {err:?}"))?;
    let face = library
        .new_memory_face2(FontData(Arc::clone(font_bytes)), 0)
        .map_err(|err| format!("加载字体失败: {err:?}"))?;
    face.set_char_size((TMP_POINT_SIZE as isize) * 64, 0, 72, 72)
        .map_err(|err| format!("设置点阵大小失败: {err:?}"))?;

    let glyph_id = resolve_glyph_id(family, font_bytes, ch)
        .ok_or_else(|| format!("无法从字体 cmap 解析 glyph id: {ch}"))?;
    // 与解析法一致用 NO_HINTING，保证 metrics 和轮廓网格对齐（hinting 会
    // 网格对齐字形、改变 width/height，导致与解析法尺寸不匹配 + 不公平对比）。
    face.load_glyph(glyph_id, LoadFlag::NO_HINTING)
        .map_err(|err| format!("按 glyph id 加载字符失败 (gid={glyph_id}): {err:?}"))?;

    let glyph = face.glyph();
    let metrics = glyph.metrics();
    let bear_x = metrics.horiBearingX as f32 / 64.0;
    let bear_y = metrics.horiBearingY as f32 / 64.0;
    let met_w = metrics.width as f32 / 64.0;
    let met_h = metrics.height as f32 / 64.0;

    // 与解析法相同的 SDF 采样网格
    let rect_left_px = bear_x.floor();
    let rect_top_px = bear_y.ceil();
    let rect_right_px = (bear_x + met_w).ceil();
    let rect_bottom_px = (bear_y - met_h).floor();
    let spread_px = TMP_SPREAD.ceil();
    let sample_left_px = rect_left_px - spread_px;
    let sample_top_px = rect_top_px + spread_px;
    let sample_right_px = rect_right_px + spread_px;
    let sample_bottom_px = rect_bottom_px - spread_px;

    let width = (sample_right_px - sample_left_px).max(1.0) as usize;
    let height = (sample_top_px - sample_bottom_px).max(1.0) as usize;
    let bearing_x = sample_left_px;
    let bearing_y = sample_top_px;

    // 光栅化到超采样分辨率
    let ss = supersample.max(1);
    let raster_w = width * ss;
    let raster_h = height * ss;
    glyph
        .render_glyph(RenderMode::Normal)
        .map_err(|err| format!("光栅化失败: {err:?}"))?;
    let bitmap = glyph.bitmap();
    let bm_w = bitmap.width() as usize;
    let bm_h = bitmap.rows() as usize;
    let bm_left = glyph.bitmap_left();
    let bm_top = glyph.bitmap_top();

    // 构建超采样覆盖率位图（inside[i] = 该像素是否在字形内部）
    let mut inside = vec![false; raster_w * raster_h];
    let mut nonzero_cov = 0usize;
    if bm_w > 0 && bm_h > 0 {
        let buffer = bitmap.buffer();
        let pitch = bitmap.pitch().abs() as usize;
        for &b in buffer.iter() {
            if b > 0 {
                nonzero_cov += 1;
            }
        }
        for ry in 0..raster_h {
            for rx in 0..raster_w {
                // 超采样像素中心在 26.6 坐标系的位置
                let px_26_6 = (sample_left_px + (rx as f32 + 0.5) / ss as f32) * 64.0;
                let py_26_6 = (sample_top_px - (ry as f32 + 0.5) / ss as f32) * 64.0;
                // 映射到 bitmap 坐标（左上角原点，Y 向下）
                let bx = ((px_26_6 / 64.0) - bm_left as f32).floor() as isize;
                let by = (bm_top as f32 - (py_26_6 / 64.0)).floor() as isize;
                if bx >= 0 && by >= 0 && (bx as usize) < bm_w && (by as usize) < bm_h {
                    let coverage = buffer[by as usize * pitch + bx as usize];
                    inside[ry * raster_w + rx] = coverage >= 128;
                }
            }
        }
    }

    let inside_count = inside.iter().filter(|&&b| b).count();
    tracing::debug!(
        ch = %ch, bm_w, bm_h, bm_left, bm_top,
        sample_left_px, sample_top_px, raster_w, raster_h,
        nonzero_cov, inside_count,
        "EDT 光栅化诊断"
    );

    // EDT 计算签名距离（单位：超采样像素）
    let sd_ss = edt::signed_distance_from_mask(&inside, raster_w, raster_h);

    // 下采样到目标分辨率 + 归一化到 [0,1] gray
    let mut pixels = vec![0u8; width * height];
    for py in 0..height {
        for px in 0..width {
            // 超采样区域平均
            let mut sum = 0.0;
            for sy in 0..ss {
                for sx in 0..ss {
                    let idx = (py * ss + sy) * raster_w + (px * ss + sx);
                    sum += sd_ss[idx];
                }
            }
            let dist_px = sum / (ss * ss) as f32 / ss as f32; // 还原到物理像素单位
            let gray = (0.5 - dist_px / (2.0 * TMP_SPREAD)).clamp(0.0, 1.0);
            pixels[py * width + px] = (gray * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    Ok(OutlineSdfGlyph {
        width,
        height,
        bearing_x,
        bearing_y,
        plane_bearing_x: bear_x,
        plane_bearing_y: bear_y,
        plane_width: met_w.max(1.0 / 64.0),
        plane_height: met_h.max(1.0 / 64.0),
        plane_advance_x: (metrics.horiAdvance as f32) / 64.0,
        pixels,
    })
}

pub fn sampling_point_size() -> f32 {
    TMP_POINT_SIZE
}
pub fn atlas_padding() -> f32 {
    TMP_ATLAS_PADDING as f32
}
pub fn sampling_spread() -> f32 {
    TMP_SPREAD
}