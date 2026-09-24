//! FreeType glyphs for uGUI `Text` layout
//! ([`sekai_profile_renderer_core::ugui_text`]).
//!
//! Glyphs are rendered the way the engine's dynamic-font cache renders them:
//! character size `S * 64` at 72 dpi, unhinted outlines, 8-bit anti-aliased
//! coverage, and the Adobe CFF engine with stem darkening off. The CFF
//! properties are set on a FreeType library of its own, so the SDF glyph path
//! never sees them.

use std::borrow::Borrow;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use freetype::bitmap::PixelMode;
use freetype::face::LoadFlag;
use freetype::freetype_sys as ffi;
use freetype::{Face, Library, RenderMode};
use lru::LruCache;
use sekai_profile_renderer_core::ugui_text::{UguiGlyph, UguiGlyphRasterizer};

use crate::sdf::outline as sdf_outline;

/// `FT_HINTING_ADOBE`, the CFF driver's `hinting-engine` value.
const HINTING_ADOBE: ffi::FT_UInt = 1;
/// Glyphs kept across renders, keyed by font file, pixel size and character.
const GLYPH_CACHE_CAPACITY: usize = 4096;

type GlyphKey = (PathBuf, u32, char);

fn glyph_cache() -> &'static Mutex<LruCache<GlyphKey, Option<Arc<UguiGlyph>>>> {
    static CACHE: OnceLock<Mutex<LruCache<GlyphKey, Option<Arc<UguiGlyph>>>>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(LruCache::new(
            NonZeroUsize::new(GLYPH_CACHE_CAPACITY).expect("glyph cache capacity > 0"),
        ))
    })
}

struct FontBytes(Arc<Vec<u8>>);

impl Borrow<[u8]> for FontBytes {
    fn borrow(&self) -> &[u8] {
        self.0.as_slice()
    }
}

/// Glyphs of one font file. The FreeType face is opened on the first glyph
/// that is not cached yet.
pub(crate) struct UguiGlyphs {
    path: PathBuf,
    bytes: Arc<Vec<u8>>,
    face: Option<Face<FontBytes>>,
    char_size: u32,
    error: Option<String>,
}

impl UguiGlyphs {
    /// Glyphs of the font file installed for `family`, or `None` when there
    /// is none.
    pub(crate) fn for_family(family: &str) -> Option<Self> {
        let path = sdf_outline::resolve_font_path(family)?;
        let bytes = sdf_outline::load_font_bytes_for_family(family)?;
        Some(Self::from_bytes(path, bytes))
    }

    fn from_bytes(path: PathBuf, bytes: Arc<Vec<u8>>) -> Self {
        Self {
            path,
            bytes,
            face: None,
            char_size: 0,
            error: None,
        }
    }

    /// The first FreeType failure, if any. A glyph that failed is reported
    /// to the layout as missing and is not cached.
    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn render(&mut self, ch: char, pixel_size: u32) -> Result<Option<UguiGlyph>, String> {
        if self.face.is_none() {
            self.face = Some(open_face(self.bytes.clone())?);
        }
        let face = self.face.as_ref().expect("face opened above");
        if self.char_size != pixel_size {
            let height = isize::try_from(u64::from(pixel_size) * 64)
                .map_err(|_| format!("pixel size {pixel_size} is out of range"))?;
            face.set_char_size(0, height, 0, 72)
                .map_err(|error| format!("set char size {pixel_size}: {error:?}"))?;
            self.char_size = pixel_size;
        }
        let Some(index) = face.get_char_index(ch as usize) else {
            return Ok(None);
        };
        face.load_glyph(index, LoadFlag::NO_HINTING)
            .map_err(|error| format!("load glyph U+{:04X}: {error:?}", u32::from(ch)))?;
        let slot = face.glyph();
        slot.render_glyph(RenderMode::Normal)
            .map_err(|error| format!("render glyph U+{:04X}: {error:?}", u32::from(ch)))?;
        let bitmap = slot.bitmap();
        let width = usize::try_from(bitmap.width()).unwrap_or(0);
        let rows = usize::try_from(bitmap.rows()).unwrap_or(0);
        let mut coverage = Vec::with_capacity(width * rows);
        if width != 0 && rows != 0 {
            if !matches!(bitmap.pixel_mode(), Ok(PixelMode::Gray)) {
                return Err(format!(
                    "glyph U+{:04X} did not render to 8-bit coverage",
                    u32::from(ch)
                ));
            }
            let pitch = bitmap.pitch();
            let stride = pitch.unsigned_abs() as usize;
            let buffer = bitmap.buffer();
            for row in 0..rows {
                // A negative pitch stores the bottom row first.
                let stored = if pitch < 0 { rows - 1 - row } else { row };
                let start = stored * stride;
                let line = buffer
                    .get(start..start + width)
                    .ok_or_else(|| format!("glyph U+{:04X} bitmap is truncated", u32::from(ch)))?;
                coverage.extend_from_slice(line);
            }
        }
        // `FT_Pos` is a C `long`, 32 bits on some targets.
        #[allow(clippy::unnecessary_cast)]
        let advance_26_6 = slot.metrics().horiAdvance as i64;
        Ok(Some(UguiGlyph {
            bitmap_left: slot.bitmap_left(),
            bitmap_top: slot.bitmap_top(),
            width: width as u32,
            rows: rows as u32,
            advance_26_6,
            coverage,
        }))
    }
}

impl UguiGlyphRasterizer for UguiGlyphs {
    fn glyph(&mut self, ch: char, pixel_size: u32) -> Option<Arc<UguiGlyph>> {
        let key = (self.path.clone(), pixel_size, ch);
        if let Some(cached) = glyph_cache()
            .lock()
            .ok()
            .and_then(|mut cache| cache.get(&key).cloned())
        {
            return cached;
        }
        match self.render(ch, pixel_size) {
            Ok(glyph) => {
                let glyph = glyph.map(Arc::new);
                if let Ok(mut cache) = glyph_cache().lock() {
                    cache.put(key, glyph.clone());
                }
                glyph
            }
            Err(error) => {
                self.error.get_or_insert(error);
                None
            }
        }
    }
}

fn open_face(bytes: Arc<Vec<u8>>) -> Result<Face<FontBytes>, String> {
    let library = Library::init().map_err(|error| format!("init FreeType: {error:?}"))?;
    let no_stem_darkening: ffi::FT_Bool = 1;
    set_property(&library, b"cff\0", b"hinting-engine\0", &HINTING_ADOBE)?;
    set_property(
        &library,
        b"cff\0",
        b"no-stem-darkening\0",
        &no_stem_darkening,
    )?;
    // The face keeps its own reference to the library.
    library
        .new_memory_face2(FontBytes(bytes), 0)
        .map_err(|error| format!("open font face: {error:?}"))
}

fn set_property<T>(
    library: &Library,
    module: &[u8],
    property: &[u8],
    value: &T,
) -> Result<(), String> {
    debug_assert!(module.ends_with(b"\0") && property.ends_with(b"\0"));
    // SAFETY: both names are NUL-terminated, and `value` points to the type
    // the driver reads for this property (`FT_UInt` for `hinting-engine`,
    // `FT_Bool` for `no-stem-darkening`). FreeType copies the value.
    let error = unsafe {
        ffi::FT_Property_Set(
            library.raw(),
            module.as_ptr().cast(),
            property.as_ptr().cast(),
            (value as *const T).cast(),
        )
    };
    if error == ffi::FT_Err_Ok {
        Ok(())
    } else {
        Err(format!(
            "set FreeType property {}: error {error}",
            String::from_utf8_lossy(&property[..property.len() - 1])
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dejavu() -> Option<UguiGlyphs> {
        UguiGlyphs::for_family("DejaVu Sans")
    }

    #[test]
    fn glyphs_render_unhinted_at_the_requested_pixel_size_and_are_cached() {
        let Some(mut glyphs) = dejavu() else {
            eprintln!("DejaVu Sans is not installed; skipping");
            return;
        };
        // Sizes no other test renders, so the cache entries are this test's own.
        let large = glyphs.glyph('H', 41).expect("H");
        let small = glyphs.glyph('H', 21).expect("H");
        assert!(large.rows > small.rows && large.width > small.width);
        assert_eq!(large.coverage.len(), (large.width * large.rows) as usize);
        assert!(large.coverage.contains(&255));
        // Unhinted advances keep their fraction of a pixel.
        assert_ne!(large.advance_26_6 % 64, 0);
        assert!(Arc::ptr_eq(&glyphs.glyph('H', 41).expect("H"), &large));
        let space = glyphs.glyph(' ', 41).expect("space");
        assert!(space.coverage.is_empty() && space.advance_26_6 > 0);
        assert_eq!(glyphs.glyph('\u{10FFFD}', 41), None);
        assert_eq!(glyphs.error(), None);
    }

    #[test]
    fn a_font_file_freetype_cannot_open_is_an_error_not_a_missing_glyph() {
        let mut glyphs = UguiGlyphs::from_bytes(
            PathBuf::from("not-a-font.otf"),
            Arc::new(b"not a font".to_vec()),
        );
        assert_eq!(glyphs.glyph('A', 40), None);
        assert!(glyphs
            .error()
            .is_some_and(|error| error.contains("open font face")));
    }
}
