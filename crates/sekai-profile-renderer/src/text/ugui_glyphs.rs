//! FreeType glyphs for uGUI `Text` layout
//! ([`sekai_profile_renderer_core::ugui_text`]).
//!
//! Glyphs are rendered by the shared
//! [`sekai_profile_renderer_core::ugui_freetype`] faces, so the native and
//! browser renderers rasterise them with the same FreeType settings. This
//! wrapper finds the font file installed for a family and keeps rendered
//! glyphs across renders.

use std::borrow::Borrow;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use lru::LruCache;
use sekai_profile_renderer_core::ugui_freetype::UguiFreeTypeFaces;
use sekai_profile_renderer_core::ugui_text::{UguiGlyph, UguiGlyphRasterizer};

use crate::sdf::outline as sdf_outline;

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
    faces: UguiFreeTypeFaces<FontBytes>,
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
            faces: UguiFreeTypeFaces::new([FontBytes(bytes)]),
            error: None,
        }
    }

    /// The first FreeType failure, if any. A glyph that failed is reported
    /// to the layout as missing and is not cached.
    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
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
        match self.faces.render(ch, pixel_size) {
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
