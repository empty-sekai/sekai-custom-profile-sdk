//! On-demand SDF glyph generation from FreeType `NO_HINTING` outlines.

mod geometry;

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use freetype::{face::LoadFlag, Library, RenderMode};
use lru::LruCache;
use sekai_profile_core::sdf_glyph::{self, CoverageBitmap, GlyphSdfGrid};
use ttf_parser::Face as TtfFace;

use self::geometry::extract_segments;

const TMP_POINT_SIZE: f32 = 75.0;
/// TextMesh Pro gradient scale: the atlas padding (5) plus one texel.
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
    // Source Han Sans is the open-licensed CJK sans installed alongside the
    // game faces. It carries the same outlines as Noto Sans CJK, the face the
    // Live Master progress digits are drawn with.
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

/// The SDF of one generated glyph.
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

static FONT_DIRS: std::sync::RwLock<Vec<PathBuf>> = std::sync::RwLock::new(Vec::new());

/// Sets the directories font files are looked up in, ahead of the
/// directories the `SEKAI_PROFILE_FONT_DIR` and `FONT_DIR` environment
/// variables name. The setting is process-wide; families resolved before it
/// are resolved again.
pub fn set_font_dirs<I>(dirs: I)
where
    I: IntoIterator,
    I::Item: Into<PathBuf>,
{
    *FONT_DIRS.write().unwrap_or_else(|error| error.into_inner()) =
        dirs.into_iter().map(Into::into).collect();
    if let Ok(mut cache) = font_path_cache().lock() {
        cache.clear();
    }
}

fn font_path_cache() -> &'static Mutex<HashMap<String, Option<PathBuf>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Capacity of the SDF glyph cache, in entries.
///
/// 4096 entries cover the common CJK, kana and Latin characters of the
/// installed families in about 25 MB, and keep the cache bounded. The key is
/// `(PathBuf, char)` rather than `(String, char)`, so alias families that name
/// the same file (such as "FZLanTingHei-DB-GBK" and "FOT-RodinNTLGPro-DB")
/// share entries.
const GLYPH_CACHE_CAPACITY: usize = 4096;

/// A codepoint of one font file, the key of the per-glyph caches.
type FontGlyphKey = (PathBuf, char);
/// Generated SDF glyphs, evicting the least recently used.
type GlyphCache = LruCache<FontGlyphKey, Arc<OutlineSdfGlyph>>;
/// Font file bytes by path; `None` records a file that could not be read.
type FontBytesCache = HashMap<PathBuf, Option<Arc<Vec<u8>>>>;
/// Glyph indices by font and codepoint; `None` records a codepoint the font
/// does not map, or a font that could not be read or parsed.
type GlyphIdCache = HashMap<FontGlyphKey, Option<u32>>;
/// Horizontal advances by font and codepoint; `None` records an advance that
/// could not be loaded.
type AdvanceCache = HashMap<FontGlyphKey, Option<f32>>;

fn glyph_cache() -> &'static Mutex<GlyphCache> {
    static CACHE: OnceLock<Mutex<GlyphCache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(LruCache::new(
            NonZeroUsize::new(GLYPH_CACHE_CAPACITY).expect("glyph cache capacity > 0"),
        ))
    })
}

fn font_bytes_cache() -> &'static Mutex<FontBytesCache> {
    static CACHE: OnceLock<Mutex<FontBytesCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn glyph_id_cache() -> &'static Mutex<GlyphIdCache> {
    static CACHE: OnceLock<Mutex<GlyphIdCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The font file of `family`: the first of its known file names found in
/// the directories [`set_font_dirs`] sets, then in those the
/// `SEKAI_PROFILE_FONT_DIR` and `FONT_DIR` environment variables name, then
/// in `assets/fonts`.
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
    let mut configured_dirs = FONT_DIRS
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    configured_dirs.extend(
        ["SEKAI_PROFILE_FONT_DIR", "FONT_DIR"]
            .into_iter()
            .filter_map(|name| std::env::var_os(name).map(PathBuf::from)),
    );
    let mut candidates = Vec::new();
    for file_name in file_names {
        for directory in &configured_dirs {
            candidates.push(directory.join(file_name));
        }
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

/// EDT generation switch and supersampling factor, read once at runtime. The
/// analytic method is the default.
///
/// `SEKAI_PROFILE_SDF_EDT` unset or `0` selects the analytic method (subpixel
/// exact, the default). `1`–`4` selects the EDT method with that
/// supersampling factor: the glyph is rasterized at that multiple of the
/// point size and then distance-transformed. Higher factors give more accurate
/// edges and slower rasterization.
fn edt_supersample() -> Option<usize> {
    static CFG: OnceLock<Option<usize>> = OnceLock::new();
    *CFG.get_or_init(|| {
        let raw = std::env::var("SEKAI_PROFILE_SDF_EDT").ok()?;
        let ss: usize = raw.trim().parse().ok()?;
        (1..=4).contains(&ss).then_some(ss)
    })
}

/// Returns the SDF of `ch` in `font_family`, generating and caching it on
/// first use.
///
/// Returns `None` when no family is given, the family's font file is not
/// found, or the glyph cannot be generated. The cache key is
/// `(PathBuf, char)` rather than `(family_name, char)`, so alias families
/// that name the same font file share one SDF.
pub fn lookup_or_generate(font_family: Option<&str>, ch: char) -> Option<Arc<OutlineSdfGlyph>> {
    let family = font_family?;
    let path = resolve_font_path(family)?;

    // Key by (font_path, char) so aliases share an entry.
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
            .or_else(|_| generate_outline_sdf(&path, ch)) // analytic fallback
            .ok()?,
        None => generate_outline_sdf(&path, ch).ok()?,
    };
    let glyph = Arc::new(glyph);
    if let Ok(mut cache) = glyph_cache().lock() {
        cache.put(key, glyph.clone());
    }
    Some(glyph)
}

/// Deterministic generation method for offline atlas builds.
///
/// Offline generation does not read `SEKAI_PROFILE_SDF_EDT` and bypasses the
/// LRU cache, so the manifest records the exact generation contract and one
/// process can build atlases with different methods.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OfflineGenerationMethod {
    Analytic,
    Edt { supersample: usize },
}

impl OfflineGenerationMethod {
    /// The generation contract written to the atlas manifest. It changes
    /// whenever the algorithm's output changes.
    pub fn contract(self) -> String {
        match self {
            Self::Analytic => "outline-analytic-v1".into(),
            Self::Edt { supersample } => {
                format!("outline-edt-v2:ss={supersample}:fallback=analytic-v1")
            }
        }
    }
}

/// Offline atlas glyph generator that keeps its FreeType library and face open.
///
/// A whole-font build walks tens of thousands of cmap codepoints; reusing the
/// face avoids reopening the font file for every glyph. The supersampled face
/// for EDT rasterization is opened on first use of each factor and reused. The
/// generator is meant for offline builds and does not share the on-demand
/// glyph cache.
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
            .ok_or_else(|| format!("no font file for family {font_family}"))?;
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
            return Err("invalid offline atlas point size".into());
        }
        if !spread.is_finite() || spread <= 0.0 {
            return Err("invalid offline atlas spread".into());
        }
        let library =
            Library::init().map_err(|err| format!("failed to initialize FreeType: {err:?}"))?;
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

    /// The returned `bool` is `true` when EDT failed and the analytic method
    /// was used instead. Callers record such fallbacks per codepoint in the
    /// manifest, so no codepoint silently uses a different method.
    pub fn generate(
        &self,
        ch: char,
        method: OfflineGenerationMethod,
    ) -> Result<(OutlineSdfGlyph, bool), String> {
        let glyph_id = self
            .face
            .get_char_index(ch as usize)
            .ok_or_else(|| format!("the font cmap has no glyph for {ch}"))?;
        let OfflineGenerationMethod::Edt { supersample } = method else {
            return generate_outline_sdf_with_face_at_spread(&self.face, glyph_id, ch, self.spread)
                .map(|glyph| (glyph, false));
        };
        if !(1..=4).contains(&supersample) {
            return Err(format!(
                "EDT supersample must be in 1..=4, got {supersample}"
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
                        format!("EDT failed: {edt_error}; analytic fallback: {analytic_error}")
                    })
            }
        }
    }
}

/// Generates the analytic and the EDT SDF of the same glyph and returns both
/// with their generation times, for comparison tools.
///
/// Bypasses the cache so every call measures a full generation. `supersample`
/// is the EDT supersampling factor.
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

/// Opens a font at 72 dpi and sets the sampling point size.
fn open_face(
    library: &Library,
    font_path: &Path,
    point_size: f32,
) -> Result<freetype::Face, String> {
    let face = library
        .new_face(font_path, 0)
        .map_err(|err| format!("failed to load the font: {err:?}"))?;
    face.set_char_size((point_size * 64.0).round() as isize, 0, 72, 72)
        .map_err(|err| format!("failed to set the pixel size: {err:?}"))?;
    Ok(face)
}

/// Glyph metrics at the sampling point size, in pixels.
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
    let library =
        Library::init().map_err(|err| format!("failed to initialize FreeType: {err:?}"))?;
    let face = open_face(&library, font_path, TMP_POINT_SIZE)?;
    let glyph_id = resolve_glyph_id(font_path, ch)
        .ok_or_else(|| format!("the font cmap has no glyph for {ch}"))?;
    generate_outline_sdf_with_face_at_spread(&face, glyph_id, ch, TMP_SPREAD)
}

/// Analytic method: the exact signed distance from the outline to every texel
/// center.
fn generate_outline_sdf_with_face_at_spread(
    face: &freetype::Face,
    glyph_id: u32,
    ch: char,
    spread: f32,
) -> Result<OutlineSdfGlyph, String> {
    face.load_glyph(glyph_id, LoadFlag::NO_BITMAP | LoadFlag::NO_HINTING)
        .map_err(|err| format!("failed to load glyph {glyph_id}: {err:?}"))?;

    let glyph = face.glyph();
    let outline = &glyph.raw().outline;
    if outline.n_contours <= 0 || outline.n_points <= 0 {
        return Err(format!("{ch} has no outline"));
    }
    // SAFETY: `outline` borrows the face's slot, which the load on the
    // preceding line filled; the face keeps its point, tag and contour arrays
    // alive until the next load on this face.
    let contours = unsafe { extract_segments(outline) };
    if contours.is_empty() {
        return Err(format!("{ch} has an empty outline"));
    }
    let metrics = SamplingMetrics::from_slot(glyph);
    let grid = metrics.grid(spread);
    Ok(metrics.into_glyph(grid, sdf_glyph::analytic_sdf(&contours, grid, spread)))
}

/// EDT SDF generation: FreeType supersampled rasterization followed by a
/// Euclidean distance transform.
///
/// Uses the same width/height/bearing grid as the analytic method; only the
/// pixel fill differs. `supersample` is the supersampling factor: the glyph is
/// rasterized at that multiple of the point size and each texel is computed
/// from `supersample`² cells. Higher factors give more accurate edges.
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
        return Err("invalid realtime EDT point size".into());
    }
    if !spread.is_finite() || spread <= 0.0 {
        return Err("invalid realtime EDT spread".into());
    }
    let path = resolve_font_path(font_family)
        .ok_or_else(|| format!("no font file for family {font_family}"))?;
    generate_outline_sdf_edt_at_point_size(&path, ch, supersample, point_size, spread)
}

fn generate_outline_sdf_edt_at_point_size(
    font_path: &Path,
    ch: char,
    supersample: usize,
    point_size: f32,
    spread: f32,
) -> Result<OutlineSdfGlyph, String> {
    let library =
        Library::init().map_err(|err| format!("failed to initialize FreeType: {err:?}"))?;
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
        .ok_or_else(|| format!("the font cmap has no glyph for {ch}"))?;
    generate_outline_sdf_edt_with_faces(
        &face,
        supersampled_face.as_ref().unwrap_or(&face),
        glyph_id,
        ch,
        supersample,
        spread,
    )
}

/// `face` is set to the sampling point size and provides the metrics and the
/// grid; `raster_face` is set to `supersample` times that size, so each pixel
/// of its coverage bitmap lines up with one supersampled cell of the grid.
fn generate_outline_sdf_edt_with_faces(
    face: &freetype::Face,
    raster_face: &freetype::Face,
    glyph_id: u32,
    ch: char,
    supersample: usize,
    spread: f32,
) -> Result<OutlineSdfGlyph, String> {
    // NO_HINTING, as in the analytic method, keeps the metrics and the outline
    // grid aligned: hinting snaps the glyph to the pixel grid and changes its
    // width and height, so it would not match the analytic size.
    face.load_glyph(glyph_id, LoadFlag::NO_HINTING)
        .map_err(|err| format!("failed to load glyph {glyph_id}: {err:?}"))?;
    let metrics = SamplingMetrics::from_slot(face.glyph());
    let grid = metrics.grid(spread);

    raster_face
        .load_glyph(glyph_id, LoadFlag::NO_HINTING)
        .map_err(|err| format!("failed to load glyph {glyph_id}: {err:?}"))?;
    let slot = raster_face.glyph();
    slot.render_glyph(RenderMode::Normal)
        .map_err(|err| format!("failed to rasterize {ch}: {err:?}"))?;
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

fn advance_cache() -> &'static Mutex<AdvanceCache> {
    static CACHE: OnceLock<Mutex<AdvanceCache>> = OnceLock::new();
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
mod tests;
