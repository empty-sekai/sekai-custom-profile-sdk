//! 基于 FreeType `NO_HINTING` 轮廓的动态 SDF glyph 生成器。

mod geometry;

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use freetype::{face::LoadFlag, Library, RenderMode};
use lru::LruCache;
use sekai_profile_renderer_core::sdf_glyph::{self, CoverageBitmap, GlyphSdfGrid};
use ttf_parser::Face as TtfFace;

use self::geometry::extract_segments;

const TMP_POINT_SIZE: f32 = 75.0;
/// TextMesh Pro 的 gradient scale：atlas padding（5）加一个 texel。
const TMP_SPREAD: f32 = 6.0;

const FONT_FILE_MAP: [(&str, &[&str]); 19] = [
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
    // Font assets of the omikuji slips, which draw with FreeType coverage
    // rather than an SDF atlas.
    ("FOT-Omikuji", &["FOT-Omikuji.otf"]),
    ("FOT-UDMinchoPro-B", &["FOT-UDMinchoPro-B.otf"]),
    // Fallback faces of the omikuji slips' text: the system UI font and the
    // faces of the Noto Sans CJK collection.
    ("Roboto", &["Roboto-Regular.ttf"]),
    ("Noto Sans CJK JP", &["NotoSansCJK-Regular.ttc"]),
    ("Noto Sans CJK KR", &["NotoSansCJK-Regular.ttc"]),
    ("Noto Sans CJK SC", &["NotoSansCJK-Regular.ttc"]),
    ("Noto Sans CJK TC", &["NotoSansCJK-Regular.ttc"]),
    // Source Han Sans is the open-licensed CJK sans shipped alongside the game
    // faces. It carries the same outlines as Noto Sans CJK, which is what the
    // Live Master progress recipe used to reach through fontconfig.
    (
        "Source Han Sans SC",
        &["SourceHanSansSC-Medium.otf", "SourceHanSansSC-Regular.otf"],
    ),
    ("SourceHanSansSC-Medium", &["SourceHanSansSC-Medium.otf"]),
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
    let configured_dirs = ["SEKAI_PROFILE_FONT_DIR", "FONT_DIR"]
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

pub fn load_font_bytes_for_family(family: &str) -> Option<Arc<Vec<u8>>> {
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
/// `SEKAI_PROFILE_SDF_EDT` 未设/为 0 → 解析法（亚像素精确，现有生产行为）。
/// 设为 1-4 → EDT 法，值即超采样因子（按该倍数的点阵光栅化后做距离变换，
/// 倍数越高边缘越精确、光栅化越慢）。
fn edt_supersample() -> Option<usize> {
    static CFG: OnceLock<Option<usize>> = OnceLock::new();
    *CFG.get_or_init(|| {
        let raw = std::env::var("SEKAI_PROFILE_SDF_EDT").ok()?;
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
/// 该入口不读取 `SEKAI_PROFILE_SDF_EDT`、不走 LRU cache，因此 manifest 可以准确记录生成契约，
/// 且同一进程可以构建不同方法的候选 atlas。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OfflineGenerationMethod {
    Analytic,
    Edt { supersample: usize },
}

impl OfflineGenerationMethod {
    /// 写入 atlas manifest 的生成契约；生成算法改变输出时随之变化。
    pub fn contract(self) -> String {
        match self {
            Self::Analytic => "outline-analytic-v1".into(),
            Self::Edt { supersample } => {
                format!("outline-edt-v2:ss={supersample}:fallback=analytic-v1")
            }
        }
    }
}

/// 持久化 FreeType library/face 的离线 atlas glyph 生成器。
///
/// 全字体构建会依次处理数万个 cmap codepoint；复用 face 避免每个 glyph 重开字体文件。
/// EDT 光栅化用的超采样 face 按超采样因子首次使用时打开并复用。
/// 该类型不进入请求期，也不共享给动态 glyph cache。
pub struct OfflineAtlasGlyphGenerator {
    face: freetype::Face,
    supersampled_faces: RefCell<Vec<(usize, freetype::Face)>>,
    library: Library,
    path: PathBuf,
    point_size: f32,
    spread: f32,
}

impl OfflineAtlasGlyphGenerator {
    pub fn new(font_family: &str) -> Result<Self, String> {
        Self::new_at_sampling(font_family, TMP_POINT_SIZE, TMP_SPREAD)
    }

    pub fn new_at_sampling(
        font_family: &str,
        point_size: f32,
        spread: f32,
    ) -> Result<Self, String> {
        let path = resolve_font_path(font_family)
            .ok_or_else(|| format!("找不到字体 family: {font_family}"))?;
        Self::new_from_path_at_sampling(&path, point_size, spread)
    }

    pub fn new_from_path(path: &Path) -> Result<Self, String> {
        Self::new_from_path_at_sampling(path, TMP_POINT_SIZE, TMP_SPREAD)
    }

    pub fn new_from_path_at_sampling(
        path: &Path,
        point_size: f32,
        spread: f32,
    ) -> Result<Self, String> {
        if !point_size.is_finite() || point_size <= 0.0 {
            return Err("离线 atlas point size 非法".into());
        }
        if !spread.is_finite() || spread <= 0.0 {
            return Err("离线 atlas spread 非法".into());
        }
        let library = Library::init().map_err(|err| format!("初始化 FreeType 失败: {err:?}"))?;
        let face = open_face(&library, path, point_size)?;
        Ok(Self {
            face,
            supersampled_faces: RefCell::new(Vec::new()),
            library,
            path: path.to_path_buf(),
            point_size,
            spread,
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
        let OfflineGenerationMethod::Edt { supersample } = method else {
            return generate_outline_sdf_with_face_at_spread(&self.face, glyph_id, ch, self.spread)
                .map(|glyph| (glyph, false));
        };
        if !(1..=4).contains(&supersample) {
            return Err(format!(
                "EDT supersample 必须在 1..=4，实际为 {supersample}"
            ));
        }
        let edt = if supersample == 1 {
            generate_outline_sdf_edt_with_faces(
                &self.face,
                &self.face,
                glyph_id,
                ch,
                supersample,
                self.spread,
            )
        } else {
            let mut faces = self.supersampled_faces.borrow_mut();
            let index = match faces.iter().position(|(factor, _)| *factor == supersample) {
                Some(index) => index,
                None => {
                    let face = open_face(
                        &self.library,
                        &self.path,
                        self.point_size * supersample as f32,
                    )?;
                    faces.push((supersample, face));
                    faces.len() - 1
                }
            };
            generate_outline_sdf_edt_with_faces(
                &self.face,
                &faces[index].1,
                glyph_id,
                ch,
                supersample,
                self.spread,
            )
        };
        match edt {
            Ok(glyph) => Ok((glyph, false)),
            Err(edt_error) => {
                generate_outline_sdf_with_face_at_spread(&self.face, glyph_id, ch, self.spread)
                    .map(|glyph| (glyph, true))
                    .map_err(|analytic_error| {
                        format!("EDT 失败: {edt_error}; 解析法回退也失败: {analytic_error}")
                    })
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

/// 以 72 dpi 打开字体并设置采样点阵大小。
fn open_face(
    library: &Library,
    font_path: &Path,
    point_size: f32,
) -> Result<freetype::Face, String> {
    let face = library
        .new_face(font_path, 0)
        .map_err(|err| format!("加载字体失败: {err:?}"))?;
    face.set_char_size((point_size * 64.0).round() as isize, 0, 72, 72)
        .map_err(|err| format!("设置点阵大小失败: {err:?}"))?;
    Ok(face)
}

/// 采样点阵下的 glyph metrics（像素）。
#[derive(Clone, Copy)]
struct SamplingMetrics {
    bearing_x: f32,
    bearing_y: f32,
    width: f32,
    height: f32,
    advance_x: f32,
}

impl SamplingMetrics {
    fn from_slot(slot: &freetype::GlyphSlot) -> Self {
        let metrics = slot.metrics();
        Self {
            bearing_x: metrics.horiBearingX as f32 / 64.0,
            bearing_y: metrics.horiBearingY as f32 / 64.0,
            width: metrics.width as f32 / 64.0,
            height: metrics.height as f32 / 64.0,
            advance_x: metrics.horiAdvance as f32 / 64.0,
        }
    }

    fn grid(self, spread: f32) -> GlyphSdfGrid {
        GlyphSdfGrid::from_metrics(
            self.bearing_x,
            self.bearing_y,
            self.width,
            self.height,
            spread,
        )
    }

    fn into_glyph(self, grid: GlyphSdfGrid, pixels: Vec<u8>) -> OutlineSdfGlyph {
        OutlineSdfGlyph {
            width: grid.width,
            height: grid.height,
            bearing_x: grid.left,
            bearing_y: grid.top,
            plane_bearing_x: self.bearing_x,
            plane_bearing_y: self.bearing_y,
            plane_width: self.width.max(1.0 / 64.0),
            plane_height: self.height.max(1.0 / 64.0),
            plane_advance_x: self.advance_x,
            pixels,
        }
    }
}

fn generate_outline_sdf(font_path: &Path, ch: char) -> Result<OutlineSdfGlyph, String> {
    let library = Library::init().map_err(|err| format!("初始化 FreeType 失败: {err:?}"))?;
    let face = open_face(&library, font_path, TMP_POINT_SIZE)?;
    let glyph_id = resolve_glyph_id(font_path, ch)
        .ok_or_else(|| format!("无法从字体 cmap 解析 glyph id: {ch}"))?;
    generate_outline_sdf_with_face_at_spread(&face, glyph_id, ch, TMP_SPREAD)
}

/// 解析法：对轮廓求每个 texel 中心的精确符号距离。
fn generate_outline_sdf_with_face_at_spread(
    face: &freetype::Face,
    glyph_id: u32,
    ch: char,
    spread: f32,
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
    let metrics = SamplingMetrics::from_slot(glyph);
    let grid = metrics.grid(spread);
    Ok(metrics.into_glyph(grid, sdf_glyph::analytic_sdf(&contours, grid, spread)))
}

/// EDT 版 SDF 生成（FreeType 超采样光栅化 + 欧几里得距离变换）。
///
/// 与解析法对齐到相同的 width/height/bearing 网格，仅像素填充算法不同。
/// `supersample` 为超采样因子：按该倍数的点阵光栅化，每个 texel 由
/// `supersample`² 个单元求得，倍数越高边缘越精确。
fn generate_outline_sdf_edt(
    font_path: &Path,
    ch: char,
    supersample: usize,
) -> Result<OutlineSdfGlyph, String> {
    generate_outline_sdf_edt_at_point_size(font_path, ch, supersample, TMP_POINT_SIZE, TMP_SPREAD)
}

/// Generate a request-local EDT glyph at the requested sampling point size.
///
/// This deliberately bypasses `glyph_cache`: callers use it when the final
/// device transform would magnify the immutable 75px atlas enough to expose
/// its texel grid. The returned metrics and bitmap are both expressed at
/// `point_size`, so consumers can preserve the authored layout while sampling
/// close to one SDF texel per device pixel.
pub(crate) fn generate_realtime_edt(
    font_family: &str,
    ch: char,
    point_size: f32,
    spread: f32,
    supersample: usize,
) -> Result<OutlineSdfGlyph, String> {
    if !point_size.is_finite() || point_size <= 0.0 {
        return Err("实时 EDT point size 非法".into());
    }
    if !spread.is_finite() || spread <= 0.0 {
        return Err("实时 EDT spread 非法".into());
    }
    let path = resolve_font_path(font_family)
        .ok_or_else(|| format!("找不到字体 family: {font_family}"))?;
    generate_outline_sdf_edt_at_point_size(&path, ch, supersample, point_size, spread)
}

fn generate_outline_sdf_edt_at_point_size(
    font_path: &Path,
    ch: char,
    supersample: usize,
    point_size: f32,
    spread: f32,
) -> Result<OutlineSdfGlyph, String> {
    let library = Library::init().map_err(|err| format!("初始化 FreeType 失败: {err:?}"))?;
    let face = open_face(&library, font_path, point_size)?;
    let supersample = supersample.max(1);
    let supersampled_face = if supersample > 1 {
        Some(open_face(
            &library,
            font_path,
            point_size * supersample as f32,
        )?)
    } else {
        None
    };
    let glyph_id = resolve_glyph_id(font_path, ch)
        .ok_or_else(|| format!("无法从字体 cmap 解析 glyph id: {ch}"))?;
    generate_outline_sdf_edt_with_faces(
        &face,
        supersampled_face.as_ref().unwrap_or(&face),
        glyph_id,
        ch,
        supersample,
        spread,
    )
}

/// `face` 处于采样点阵，提供 metrics 与网格；`raster_face` 处于
/// `supersample` 倍点阵，其覆盖率位图的像素与网格的超采样单元一一对齐。
fn generate_outline_sdf_edt_with_faces(
    face: &freetype::Face,
    raster_face: &freetype::Face,
    glyph_id: u32,
    ch: char,
    supersample: usize,
    spread: f32,
) -> Result<OutlineSdfGlyph, String> {
    // 与解析法一致用 NO_HINTING，保证 metrics 和轮廓网格对齐（hinting 会
    // 网格对齐字形、改变 width/height，导致与解析法尺寸不匹配）。
    face.load_glyph(glyph_id, LoadFlag::NO_HINTING)
        .map_err(|err| format!("按 glyph id 加载字符失败 (gid={glyph_id}): {err:?}"))?;
    let metrics = SamplingMetrics::from_slot(face.glyph());
    let grid = metrics.grid(spread);

    raster_face
        .load_glyph(glyph_id, LoadFlag::NO_HINTING)
        .map_err(|err| format!("按 glyph id 加载字符失败 (gid={glyph_id}): {err:?}"))?;
    let slot = raster_face.glyph();
    slot.render_glyph(RenderMode::Normal)
        .map_err(|err| format!("光栅化失败 ({ch}): {err:?}"))?;
    let bitmap = slot.bitmap();
    let width = bitmap.width().max(0) as usize;
    let rows = bitmap.rows().max(0) as usize;
    let coverage = CoverageBitmap {
        buffer: if width > 0 && rows > 0 {
            bitmap.buffer()
        } else {
            &[]
        },
        width,
        rows,
        pitch: bitmap.pitch().unsigned_abs() as usize,
        left: slot.bitmap_left(),
        top: slot.bitmap_top(),
    };
    let pixels = sdf_glyph::edt_sdf(&coverage, grid, supersample, spread);
    Ok(metrics.into_glyph(grid, pixels))
}

pub fn sampling_point_size() -> f32 {
    TMP_POINT_SIZE
}
pub fn sampling_spread() -> f32 {
    TMP_SPREAD
}

fn advance_cache() -> &'static Mutex<HashMap<(PathBuf, char), Option<f32>>> {
    static CACHE: OnceLock<Mutex<HashMap<(PathBuf, char), Option<f32>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Horizontal advance for one codepoint, in the same units as
/// [`OutlineSdfGlyph::plane_advance_x`].
///
/// Glyphs with no outline — the space being the one that matters in practice —
/// cannot produce an SDF, so [`lookup_or_generate`] rejects them and their
/// advance was previously taken from another engine. FreeType has the advance in
/// `hmtx` regardless of whether there is anything to draw, so read it directly
/// using the same face setup the SDF path uses (`TMP_POINT_SIZE` at 72 dpi,
/// `NO_HINTING`) to keep both sources on one metric.
pub fn glyph_advance_x(font_family: Option<&str>, ch: char) -> Option<f32> {
    let family = font_family?;
    let path = resolve_font_path(family)?;

    let key = (path.clone(), ch);
    if let Some(cached) = advance_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(&key).copied())
    {
        return cached;
    }

    let advance = load_glyph_advance_x(&path, ch);
    if let Ok(mut cache) = advance_cache().lock() {
        cache.insert(key, advance);
    }
    advance
}

fn load_glyph_advance_x(font_path: &Path, ch: char) -> Option<f32> {
    let library = Library::init().ok()?;
    let face = library.new_face(font_path, 0).ok()?;
    face.set_char_size((TMP_POINT_SIZE as isize) * 64, 0, 72, 72)
        .ok()?;
    let glyph_id = resolve_glyph_id(font_path, ch)?;
    face.load_glyph(glyph_id, LoadFlag::NO_BITMAP | LoadFlag::NO_HINTING)
        .ok()?;
    let metrics = face.glyph().raw().metrics;
    Some((metrics.horiAdvance as f32) / 64.0)
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
    fn every_omikuji_font_asset_names_its_font_file() {
        for prefab in sekai_profile_renderer_core::omikuji::PREFABS {
            let family = prefab.font_family;
            let files = FONT_FILE_MAP
                .iter()
                .find_map(|(key, files)| (*key == family).then_some(*files));
            assert_eq!(
                files,
                Some(&[format!("{family}.otf").as_str()][..]),
                "{}",
                prefab.bundle
            );
        }
    }

    #[test]
    fn every_omikuji_fallback_face_names_its_font_file() {
        use sekai_profile_renderer_core::omikuji::{CjkFallback, LATIN_FALLBACK_FAMILY};
        let files = |family: &str| {
            FONT_FILE_MAP
                .iter()
                .find_map(|(key, files)| (*key == family).then_some(*files))
        };
        assert_eq!(
            files(LATIN_FALLBACK_FAMILY),
            Some(&["Roboto-Regular.ttf"][..])
        );
        for cjk in CjkFallback::ALL {
            assert_eq!(
                files(cjk.family()),
                Some(&["NotoSansCJK-Regular.ttc"][..]),
                "{cjk:?}"
            );
        }
    }

    #[test]
    fn persistent_offline_face_matches_one_shot_generation() {
        let Some((family, path)) = ["FZLanTingHei-DB-GBK", "DejaVu Sans"]
            .into_iter()
            .find_map(|family| resolve_font_path(family).map(|path| (family, path)))
        else {
            eprintln!("skipping: no test family is installed");
            return;
        };
        let generator = OfflineAtlasGlyphGenerator::new(family).expect("offline generator");
        for ch in ['A', 'g'] {
            let analytic_one_shot = generate_outline_sdf(&path, ch).expect("analytic one-shot");
            let (analytic_persistent, analytic_fallback) = generator
                .generate(ch, OfflineGenerationMethod::Analytic)
                .expect("analytic persistent");
            assert!(!analytic_fallback);
            assert_glyph_exact(&analytic_persistent, &analytic_one_shot);

            for supersample in [1, 2, 4] {
                let edt_one_shot =
                    generate_outline_sdf_edt(&path, ch, supersample).expect("EDT one-shot");
                let (edt_persistent, edt_fallback) = generator
                    .generate(ch, OfflineGenerationMethod::Edt { supersample })
                    .expect("EDT persistent");
                assert!(!edt_fallback);
                assert_glyph_exact(&edt_persistent, &edt_one_shot);
            }
        }
    }

    #[test]
    fn realtime_edt_parentheses_scale_the_source_grid_without_cache_reuse() {
        if resolve_font_path("FZLanTingHei-DB-GBK").is_none() {
            eprintln!("skipping: FONT_DIR does not provide the test family");
            return;
        }

        let family = "FZLanTingHei-DB-GBK";
        for ch in ['(', ')'] {
            let normal = generate_realtime_edt(family, ch, 75.0, 6.0, 2).expect("normal EDT");
            let huge = generate_realtime_edt(family, ch, 300.0, 24.0, 2).expect("huge EDT");
            assert!(huge.width() > normal.width() * 2);
            assert!(huge.height() > normal.height() * 2);
            assert!(huge.pixels().iter().any(|&value| value != 0));
        }
    }

    /// Mean and maximum absolute gray difference over the texels where the
    /// exact field is not saturated, i.e. inside the encoded distance band.
    fn band_error(actual: &OutlineSdfGlyph, exact: &OutlineSdfGlyph) -> (f32, f32) {
        assert_eq!((actual.width, actual.height), (exact.width, exact.height));
        let mut sum = 0.0f32;
        let mut max = 0.0f32;
        let mut count = 0usize;
        for (&value, &reference) in actual.pixels.iter().zip(&exact.pixels) {
            if reference == 0 || reference == 255 {
                continue;
            }
            let error = (f32::from(value) - f32::from(reference)).abs();
            sum += error;
            max = max.max(error);
            count += 1;
        }
        assert!(
            count > 0,
            "the glyph must have texels inside the distance band"
        );
        (sum / count as f32, max)
    }

    #[test]
    fn edt_supersampling_converges_on_the_analytic_field() {
        let family = "DejaVu Sans";
        let Some(path) = resolve_font_path(family) else {
            eprintln!("skipping: DejaVu Sans is not installed");
            return;
        };
        for ch in ['O', 'a', 'g'] {
            let exact = generate_outline_sdf(&path, ch).expect("analytic glyph");
            let error = |supersample| {
                let glyph = generate_outline_sdf_edt(&path, ch, supersample).expect("EDT glyph");
                band_error(&glyph, &exact)
            };
            let (mean_2, _) = error(2);
            let (mean_4, max_4) = error(4);
            // Real supersampling resolves the edge more finely each time the
            // factor doubles; replicating a 1x mask would leave it unchanged.
            assert!(
                mean_4 < mean_2 * 0.7,
                "{ch}: ss4 mean {mean_4} is not clearly below ss2 mean {mean_2}"
            );
            assert!(mean_4 < 2.5, "{ch}: ss4 mean gray error {mean_4}");
            assert!(max_4 < 10.0, "{ch}: ss4 max gray error {max_4}");
        }
    }

    #[test]
    fn realtime_edt_rejects_non_positive_distance_spread() {
        assert!(generate_realtime_edt("unused", '(', 300.0, 0.0, 2).is_err());
    }
}
