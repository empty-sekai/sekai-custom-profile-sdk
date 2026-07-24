//! 基于 FreeType `NO_HINTING` 轮廓的动态 SDF glyph 生成器。

mod edt;
mod geometry;

use std::collections::HashMap;
use std::fs;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use allium_renderer_core::sdf_geometry::{AnalyticDistanceField, Vec2};
use freetype::{face::LoadFlag, Library, RenderMode};
use lru::LruCache;
use ttf_parser::Face as TtfFace;

use self::geometry::extract_segments;

const TMP_POINT_SIZE: f32 = 75.0;
const TMP_ATLAS_PADDING: usize = 5;
const TMP_SPREAD: f32 = 6.0;

const FONT_FILE_MAP: [(&str, &[&str]); 10] = [
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
    (
        "DejaVu Sans",
        &[
            "DejaVuSans.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ],
    ),
    (
        "DejaVuSans",
        &[
            "DejaVuSans.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ],
    ),
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

/// SDF glyph 缓存容量（条目数上限）。
///
/// 8 个字体 family × 常用 CJK/假名/拉丁字符集，4096 条目 ~25 MB，
/// 远小于无界 HashMap 的数百 MB 增长风险。
/// key 使用 `(PathBuf, char)` 而非 `(String, char)`，
/// 消除别名 family（如 "FZLanTingHei-DB-GBK" 与 "FOT-RodinNTLGPro-DB" 指向同一文件）的重复缓存。
const GLYPH_CACHE_CAPACITY: usize = 4096;

fn glyph_cache() -> &'static Mutex<LruCache<(PathBuf, char), Arc<OutlineSdfGlyph>>> {
    static CACHE: OnceLock<Mutex<LruCache<(PathBuf, char), Arc<OutlineSdfGlyph>>>> =
        OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(LruCache::new(
            NonZeroUsize::new(GLYPH_CACHE_CAPACITY).expect("glyph cache capacity > 0"),
        ))
    })
}

fn font_bytes_cache() -> &'static Mutex<HashMap<PathBuf, Option<Arc<Vec<u8>>>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Option<Arc<Vec<u8>>>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn glyph_id_cache() -> &'static Mutex<HashMap<(PathBuf, char), Option<u32>>> {
    static CACHE: OnceLock<Mutex<HashMap<(PathBuf, char), Option<u32>>>> = OnceLock::new();
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
    let candidates = font_path_candidates(file_names);

    let found = candidates.into_iter().find(|path| path.exists());
    if let Ok(mut cache) = font_path_cache().lock() {
        cache.insert(family.to_string(), found.clone());
    }
    found
}

fn font_path_candidates(file_names: &[&str]) -> Vec<PathBuf> {
    let configured_dirs = ["SCAPUS_FONT_DIR", "FONT_DIR"]
        .into_iter()
        .filter_map(|name| std::env::var_os(name).map(PathBuf::from))
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    for file_name in file_names {
        for directory in &configured_dirs {
            candidates.push(directory.join(file_name));
        }
        candidates.push(PathBuf::from("/usr/share/fonts/custom").join(file_name));
        candidates.push(PathBuf::from("assets/fonts").join(file_name));
    }
    candidates
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
    let path = resolve_font_path(family)?;
    load_font_bytes(&path)
}

fn resolve_glyph_id(font_path: &Path, ch: char) -> Option<u32> {
    let key = (font_path.to_path_buf(), ch);
    if let Some(cached) = glyph_id_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(&key).cloned())
    {
        return cached;
    }
    let resolved = load_font_bytes(font_path).and_then(|bytes| {
        let face = TtfFace::parse(bytes.as_slice(), 0).ok()?;
        face.glyph_index(ch).map(|gid| gid.0 as u32)
    });
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
///
/// 内部用 `(PathBuf, char)` 作为缓存 key，而非 `(family_name, char)`，
/// 避免别名 family（指向同一字体文件的不同名称）产生重复 SDF。
pub fn lookup_or_generate(font_family: Option<&str>, ch: char) -> Option<Arc<OutlineSdfGlyph>> {
    let family = font_family?;
    let path = resolve_font_path(family)?;

    // 用 (font_path, char) 作为 key，消除别名重复
    let key = (path.clone(), ch);
    if let Some(cached) = glyph_cache()
        .lock()
        .ok()
        .and_then(|mut cache| cache.get(&key).cloned())
    {
        return Some(cached);
    }

    let glyph = match edt_supersample() {
        Some(ss) => generate_outline_sdf_edt(&path, ch, ss)
            .or_else(|_| generate_outline_sdf(&path, ch)) // EDT 失败回退解析法
            .ok()?,
        None => generate_outline_sdf(&path, ch).ok()?,
    };
    let glyph = Arc::new(glyph);
    if let Ok(mut cache) = glyph_cache().lock() {
        cache.put(key, glyph.clone());
    }
    Some(glyph)
}

/// 离线 atlas 构建使用的确定性生成方法。
///
/// 该入口不读取 `SCAPUS_SDF_EDT`、不走 LRU cache，因此 manifest 可以准确记录生成契约，
/// 且同一进程可以构建不同方法的候选 atlas。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OfflineGenerationMethod {
    Analytic,
    Edt { supersample: usize },
}

/// 持久化 FreeType library/face 的离线 atlas glyph 生成器。
///
/// 全字体构建会依次处理数万个 cmap codepoint；复用 face 避免每个 glyph 重开字体文件。
/// 该类型不进入请求期，也不共享给动态 glyph cache。
pub struct OfflineAtlasGlyphGenerator {
    // Face 先声明以确保它先于最后一个 Library owner 释放。
    face: freetype::Face,
    _library: Library,
}

impl OfflineAtlasGlyphGenerator {
    pub fn new(font_family: &str) -> Result<Self, String> {
        let path = resolve_font_path(font_family)
            .ok_or_else(|| format!("找不到字体 family: {font_family}"))?;
        Self::new_from_path(&path)
    }

    pub fn new_from_path(path: &Path) -> Result<Self, String> {
        let library = Library::init().map_err(|err| format!("初始化 FreeType 失败: {err:?}"))?;
        let face = library
            .new_face(path, 0)
            .map_err(|err| format!("加载字体失败: {err:?}"))?;
        face.set_char_size((TMP_POINT_SIZE as isize) * 64, 0, 72, 72)
            .map_err(|err| format!("设置点阵大小失败: {err:?}"))?;
        Ok(Self {
            face,
            _library: library,
        })
    }

    /// 返回值中的 `bool` 表示 EDT 是否失败并回退到解析法。回退必须逐 codepoint
    /// 记录到 manifest，不能悄悄混入另一种生成方法。
    pub fn generate(
        &self,
        ch: char,
        method: OfflineGenerationMethod,
    ) -> Result<(OutlineSdfGlyph, bool), String> {
        let glyph_id = self
            .face
            .get_char_index(ch as usize)
            .ok_or_else(|| format!("无法从字体 cmap 解析 glyph id: {ch}"))?;
        match method {
            OfflineGenerationMethod::Analytic => {
                generate_outline_sdf_with_face(&self.face, glyph_id, ch).map(|glyph| (glyph, false))
            }
            OfflineGenerationMethod::Edt { supersample } => {
                if !(1..=4).contains(&supersample) {
                    return Err(format!(
                        "EDT supersample 必须在 1..=4，实际为 {supersample}"
                    ));
                }
                match generate_outline_sdf_edt_with_face(&self.face, glyph_id, ch, supersample) {
                    Ok(glyph) => Ok((glyph, false)),
                    Err(edt_error) => generate_outline_sdf_with_face(&self.face, glyph_id, ch)
                        .map(|glyph| (glyph, true))
                        .map_err(|analytic_error| {
                            format!("EDT 失败: {edt_error}; 解析法回退也失败: {analytic_error}")
                        }),
                }
            }
        }
    }
}

/// 对比工具专用：生成同一 glyph 的解析法 vs EDT 版 SDF，返回两者 + 耗时。
///
/// 不走缓存，每次都重新生成以测量真实性能。`supersample` 为 EDT 超采样因子。
#[cfg(feature = "dev")]
pub fn benchmark_methods(
    font_family: &str,
    ch: char,
    supersample: usize,
) -> Option<(
    Arc<OutlineSdfGlyph>,
    std::time::Duration,
    Arc<OutlineSdfGlyph>,
    std::time::Duration,
)> {
    let path = resolve_font_path(font_family)?;

    let t0 = std::time::Instant::now();
    let analytic = Arc::new(generate_outline_sdf(&path, ch).ok()?);
    let analytic_dur = t0.elapsed();

    let t1 = std::time::Instant::now();
    let edt = Arc::new(generate_outline_sdf_edt(&path, ch, supersample).ok()?);
    let edt_dur = t1.elapsed();

    Some((analytic, analytic_dur, edt, edt_dur))
}

fn generate_outline_sdf(font_path: &Path, ch: char) -> Result<OutlineSdfGlyph, String> {
    let library = Library::init().map_err(|err| format!("初始化 FreeType 失败: {err:?}"))?;
    let face = library
        .new_face(font_path, 0)
        .map_err(|err| format!("加载字体失败: {err:?}"))?;
    face.set_char_size((TMP_POINT_SIZE as isize) * 64, 0, 72, 72)
        .map_err(|err| format!("设置点阵大小失败: {err:?}"))?;

    let glyph_id = resolve_glyph_id(font_path, ch)
        .ok_or_else(|| format!("无法从字体 cmap 解析 glyph id: {ch}"))?;
    generate_outline_sdf_with_face(&face, glyph_id, ch)
}

fn generate_outline_sdf_with_face(
    face: &freetype::Face,
    glyph_id: u32,
    ch: char,
) -> Result<OutlineSdfGlyph, String> {
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

    let distance_field = AnalyticDistanceField::new(&contours);
    let mut pixels = vec![0u8; width * height];
    for py in 0..height {
        for px in 0..width {
            let point = Vec2::new(
                rect_left_26_6 + (px as f32 + 0.5) * 64.0,
                rect_top_26_6 - (py as f32 + 0.5) * 64.0,
            );
            let signed_distance_px = distance_field.signed_distance(point) / 64.0;
            let gray = (0.5 - signed_distance_px / (2.0 * TMP_SPREAD)).clamp(0.0, 1.0);
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
    font_path: &Path,
    ch: char,
    supersample: usize,
) -> Result<OutlineSdfGlyph, String> {
    let library = Library::init().map_err(|err| format!("初始化 FreeType 失败: {err:?}"))?;
    let face = library
        .new_face(font_path, 0)
        .map_err(|err| format!("加载字体失败: {err:?}"))?;
    face.set_char_size((TMP_POINT_SIZE as isize) * 64, 0, 72, 72)
        .map_err(|err| format!("设置点阵大小失败: {err:?}"))?;

    let glyph_id = resolve_glyph_id(font_path, ch)
        .ok_or_else(|| format!("无法从字体 cmap 解析 glyph id: {ch}"))?;
    generate_outline_sdf_edt_with_face(&face, glyph_id, ch, supersample)
}

fn generate_outline_sdf_edt_with_face(
    face: &freetype::Face,
    glyph_id: u32,
    ch: char,
    supersample: usize,
) -> Result<OutlineSdfGlyph, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_glyph_exact(actual: &OutlineSdfGlyph, expected: &OutlineSdfGlyph) {
        assert_eq!(
            (actual.width, actual.height),
            (expected.width, expected.height)
        );
        assert_eq!(actual.bearing_x.to_bits(), expected.bearing_x.to_bits());
        assert_eq!(actual.bearing_y.to_bits(), expected.bearing_y.to_bits());
        assert_eq!(
            actual.plane_bearing_x.to_bits(),
            expected.plane_bearing_x.to_bits()
        );
        assert_eq!(
            actual.plane_bearing_y.to_bits(),
            expected.plane_bearing_y.to_bits()
        );
        assert_eq!(actual.plane_width.to_bits(), expected.plane_width.to_bits());
        assert_eq!(
            actual.plane_height.to_bits(),
            expected.plane_height.to_bits()
        );
        assert_eq!(
            actual.plane_advance_x.to_bits(),
            expected.plane_advance_x.to_bits()
        );
        assert_eq!(actual.pixels, expected.pixels);
    }

    #[test]
    #[ignore = "static font is not shipped in the OSS repository"]
    fn persistent_offline_face_matches_one_shot_generation() {
        let family = "FZLanTingHei-DB-GBK";
        let path = resolve_font_path(family).expect("test font must exist");
        let generator = OfflineAtlasGlyphGenerator::new(family).expect("offline generator");
        for ch in ['A', '一'] {
            let analytic_one_shot = generate_outline_sdf(&path, ch).expect("analytic one-shot");
            let (analytic_persistent, analytic_fallback) = generator
                .generate(ch, OfflineGenerationMethod::Analytic)
                .expect("analytic persistent");
            assert!(!analytic_fallback);
            assert_glyph_exact(&analytic_persistent, &analytic_one_shot);

            let edt_one_shot = generate_outline_sdf_edt(&path, ch, 2).expect("EDT2 one-shot");
            let (edt_persistent, edt_fallback) = generator
                .generate(ch, OfflineGenerationMethod::Edt { supersample: 2 })
                .expect("EDT2 persistent");
            assert!(!edt_fallback);
            assert_glyph_exact(&edt_persistent, &edt_one_shot);
        }
    }
}
