//! FreeType glyphs for uGUI `Text` layout
//! ([`sekai_profile_renderer_core::ugui_text`]).
//!
//! Glyphs are rendered by the shared
//! [`sekai_profile_renderer_core::ugui_freetype`] faces, so the native and
//! browser renderers rasterise them with the same FreeType settings. This
//! wrapper finds the font file installed for each family of a text's face
//! chain and keeps rendered glyphs across renders.

use std::borrow::Borrow;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use lru::LruCache;
use sekai_profile_renderer_core::ugui_freetype::UguiFreeTypeFaces;
use sekai_profile_renderer_core::ugui_text::{UguiFontFace, UguiGlyph, UguiGlyphRasterizer};

use crate::sdf::outline as sdf_outline;

/// Glyphs kept across renders, keyed by face chain, pixel size and
/// character.
const GLYPH_CACHE_CAPACITY: usize = 4096;

/// Font files and face indices of a face chain, in order.
type ChainKey = Arc<[(PathBuf, u32)]>;
type GlyphKey = (ChainKey, u32, char);

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

/// A face of an installed font file.
pub(crate) struct FaceFile {
    pub path: PathBuf,
    pub bytes: Arc<Vec<u8>>,
    pub face_index: u32,
}

/// Glyphs of a face chain. Each FreeType face is opened on the first glyph
/// that is not cached yet and needs it.
pub(crate) struct UguiGlyphs {
    chain: ChainKey,
    faces: UguiFreeTypeFaces<FontBytes>,
    error: Option<String>,
}

impl UguiGlyphs {
    /// Glyphs of `chain`, each face from the font file installed for its
    /// family. `Err` names the first family that has no installed file.
    pub(crate) fn for_chain(chain: &[UguiFontFace]) -> Result<Self, String> {
        chain
            .iter()
            .map(|face| {
                let path = sdf_outline::resolve_font_path(&face.family);
                let bytes = sdf_outline::load_font_bytes_for_family(&face.family);
                match (path, bytes) {
                    (Some(path), Some(bytes)) => Ok(FaceFile {
                        path,
                        bytes,
                        face_index: face.face_index,
                    }),
                    _ => Err(face.family.clone()),
                }
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Self::from_files)
    }

    /// Glyphs of the faces of `files`, tried in order.
    pub(crate) fn from_files(files: Vec<FaceFile>) -> Self {
        let chain = files
            .iter()
            .map(|file| (file.path.clone(), file.face_index))
            .collect::<Vec<_>>()
            .into();
        Self {
            chain,
            faces: UguiFreeTypeFaces::new(
                files
                    .into_iter()
                    .map(|file| (FontBytes(file.bytes), file.face_index)),
            ),
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
        let key = (self.chain.clone(), pixel_size, ch);
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
pub(crate) mod tests {
    use super::*;

    /// The synthetic fonts of the uGUI text fixtures, shared with the
    /// browser renderer's tests.
    pub(crate) fn fixture_file(name: &str) -> Option<(PathBuf, Arc<Vec<u8>>)> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../allium-renderer-wasm/scripts/test/fixtures")
            .join(name);
        let bytes = std::fs::read(&path).ok()?;
        Some((path, Arc::new(bytes)))
    }

    fn face(family: &str, face_index: u32) -> UguiFontFace {
        UguiFontFace {
            family: family.into(),
            face_index,
        }
    }

    fn dejavu() -> Option<UguiGlyphs> {
        UguiGlyphs::for_chain(&[face("DejaVu Sans", 0)]).ok()
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
        let mut glyphs = UguiGlyphs::from_files(vec![FaceFile {
            path: PathBuf::from("not-a-font.otf"),
            bytes: Arc::new(b"not a font".to_vec()),
            face_index: 0,
        }]);
        assert_eq!(glyphs.glyph('A', 40), None);
        assert!(glyphs
            .error()
            .is_some_and(|error| error.contains("open font face")));
    }

    #[test]
    fn a_face_chain_needs_every_family_installed() {
        // The first family without a file is named, even when later ones
        // are missing too.
        let chain = [
            face("DejaVu Sans", 0),
            face("Uninstalled Fallback", 0),
            face("Another Uninstalled Fallback", 3),
        ];
        match UguiGlyphs::for_chain(&chain) {
            Err(family) if sdf_outline::resolve_font_path("DejaVu Sans").is_some() => {
                assert_eq!(family, "Uninstalled Fallback");
            }
            Err(family) => assert_eq!(family, "DejaVu Sans"),
            Ok(_) => panic!("a chain with uninstalled families resolved"),
        }
    }

    #[test]
    fn glyphs_come_from_the_first_face_of_the_chain_that_maps_them() {
        let (Some(primary), Some(latin), Some(cjk)) = (
            fixture_file("ugui-fixture.otf"),
            fixture_file("ugui-fixture-latin.ttf"),
            fixture_file("ugui-fixture-cjk.ttc"),
        ) else {
            eprintln!("the uGUI fixture fonts are not in the workspace; skipping");
            return;
        };
        let file = |(path, bytes): &(PathBuf, Arc<Vec<u8>>), face_index| FaceFile {
            path: path.clone(),
            bytes: bytes.clone(),
            face_index,
        };
        let single = |file: FaceFile, ch| {
            UguiFreeTypeFaces::new([(FontBytes(file.bytes), file.face_index)])
                .render(ch, 33)
                .expect("render")
        };
        let chain = |face_index| {
            UguiGlyphs::from_files(vec![
                file(&primary, 0),
                file(&latin, 0),
                file(&cjk, face_index),
            ])
        };
        let mut sc = chain(2);
        // U+0041 is the primary's, U+0042 the Latin face's although the
        // collection maps it too, and U+5173 the collection face's.
        assert_eq!(
            sc.glyph('A', 33).as_deref(),
            single(file(&primary, 0), 'A').as_ref()
        );
        assert_eq!(
            sc.glyph('B', 33).as_deref(),
            single(file(&latin, 0), 'B').as_ref()
        );
        assert_eq!(
            sc.glyph('关', 33).as_deref(),
            single(file(&cjk, 2), '关').as_ref()
        );
        // No face maps U+005A.
        assert_eq!(sc.glyph('Z', 33), None);
        // Another face of the collection is another chain.
        let mut jp = chain(0);
        let jp_glyph = jp.glyph('关', 33).expect("JP glyph");
        assert_eq!(Some(&*jp_glyph), single(file(&cjk, 0), '关').as_ref());
        assert_ne!(Some(jp_glyph), sc.glyph('关', 33));
        assert_eq!((sc.error(), jp.error()), (None, None));
    }

    /// `value` as JSON, with `f32` values in their shortest decimal form.
    fn json_of(value: &impl serde::Serialize) -> serde_json::Value {
        serde_json::from_str(&serde_json::to_string(value).expect("serialise")).expect("JSON")
    }

    #[test]
    fn native_glyphs_lay_out_the_fallback_fixture_as_the_browser_does() {
        use sekai_profile_renderer_core::ugui_text::{self, UguiTextSource};

        let Some((_, fixture)) = fixture_file("ugui-fixture-fallback-layout.json") else {
            eprintln!("the uGUI fixtures are not in the workspace; skipping");
            return;
        };
        let fixture: serde_json::Value = serde_json::from_slice(&fixture).expect("fixture JSON");
        let fonts = fixture["request"]["fonts"].as_array().expect("fonts");
        let layout = &fixture["layout"];
        let texts = fixture["request"]["texts"].as_array().expect("texts");
        assert_eq!(
            texts.len(),
            layout["texts"].as_array().expect("layout").len()
        );
        for (text, laid_out) in texts.iter().zip(layout["texts"].as_array().expect("texts")) {
            let source: UguiTextSource =
                serde_json::from_value(text["source"].clone()).expect("source");
            let files = source
                .face_chain()
                .into_iter()
                .map(|face| {
                    let font = fonts
                        .iter()
                        .find(|font| font["family"] == face.family.as_str())
                        .expect("fixture font");
                    let (path, bytes) =
                        fixture_file(font["file"].as_str().expect("file")).expect("font file");
                    FaceFile {
                        path,
                        bytes,
                        face_index: face.face_index,
                    }
                })
                .collect();
            let mut glyphs = UguiGlyphs::from_files(files);
            let mesh = ugui_text::layout(&source, ugui_text::CARD_PIXELS_PER_UNIT, &mut glyphs);
            assert_eq!(glyphs.error(), None);
            let drawn = mesh
                .quads
                .iter()
                .filter_map(|quad| quad.glyph.as_ref().map(|glyph| (quad, glyph)))
                .collect::<Vec<_>>();
            let recorded = laid_out["quads"].as_array().expect("quads");
            assert_eq!(drawn.len(), recorded.len(), "{}", text["id"]);
            for ((quad, glyph), recorded) in drawn.into_iter().zip(recorded) {
                assert_eq!(
                    json_of(&quad.corners),
                    recorded["corners"],
                    "{}",
                    text["id"]
                );
                let placement =
                    &layout["glyphs"][recorded["glyph"].as_u64().expect("glyph") as usize];
                assert_eq!(
                    serde_json::json!([
                        glyph.bitmap_left,
                        glyph.bitmap_top,
                        glyph.width,
                        glyph.rows,
                        glyph.advance_26_6
                    ]),
                    serde_json::json!([
                        placement["bitmapLeft"],
                        placement["bitmapTop"],
                        placement["width"],
                        placement["rows"],
                        placement["advance26d6"]
                    ]),
                    "{}",
                    text["id"]
                );
            }
            assert_eq!(
                json_of(&mesh.preferred_width),
                laid_out["preferredWidth"],
                "{}",
                text["id"]
            );
        }
    }
}
