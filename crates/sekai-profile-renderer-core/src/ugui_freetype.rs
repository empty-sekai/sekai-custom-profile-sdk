//! FreeType glyphs for uGUI text layout ([`crate::ugui_text`]), shared by the
//! native and browser renderers.
//!
//! Glyphs are rendered the way the engine's dynamic-font cache renders them:
//! character size `S * 64` at [`DPI`], unhinted outlines ([`LOAD_FLAGS`]),
//! 8-bit anti-aliased coverage ([`RENDER_MODE`]), and the Adobe CFF engine
//! with stem darkening off. The CFF properties are set on a FreeType library
//! of its own, so no other FreeType user sees them.
//!
//! [`UguiFreeTypeFaces`] holds an ordered list of font files. A character is
//! rendered from the first face whose character map contains it.

use std::borrow::Borrow;
use std::sync::Arc;

use freetype::bitmap::PixelMode;
use freetype::face::LoadFlag;
use freetype::freetype_sys as ffi;
use freetype::{Face, Library, RenderMode};

use crate::ugui_text::{UguiGlyph, UguiGlyphRasterizer};

/// Resolution of the character size, in dots per inch on both axes.
pub const DPI: u32 = 72;
/// Glyphs are loaded unhinted.
pub const LOAD_FLAGS: LoadFlag = LoadFlag::NO_HINTING;
/// Glyphs are rendered to 8-bit anti-aliased coverage.
pub const RENDER_MODE: RenderMode = RenderMode::Normal;
/// `FT_HINTING_ADOBE`, the CFF driver's `hinting-engine` value.
pub const CFF_HINTING_ENGINE: ffi::FT_UInt = 1;
/// The CFF driver's `no-stem-darkening` value.
pub const CFF_NO_STEM_DARKENING: ffi::FT_Bool = 1;

/// Character height in 26.6 fixed point for glyphs of `pixel_size` pixels.
pub fn char_height_26_6(pixel_size: u32) -> i64 {
    i64::from(pixel_size) * 64
}

/// Font files in fallback order, each opened as a FreeType face on the first
/// glyph it is asked for.
pub struct UguiFreeTypeFaces<B> {
    library: Option<Library>,
    faces: Vec<FaceSlot<B>>,
    error: Option<String>,
}

struct FaceSlot<B> {
    /// The font file until its face is opened.
    bytes: Option<B>,
    face: Option<Face<B>>,
    /// Pixel size the face is set to; 0 before the first glyph.
    char_size: u32,
}

impl<B: Borrow<[u8]>> UguiFreeTypeFaces<B> {
    /// Faces of `fonts`, tried in order.
    pub fn new(fonts: impl IntoIterator<Item = B>) -> Self {
        Self {
            library: None,
            faces: fonts
                .into_iter()
                .map(|bytes| FaceSlot {
                    bytes: Some(bytes),
                    face: None,
                    char_size: 0,
                })
                .collect(),
            error: None,
        }
    }

    /// The first FreeType failure seen through [`UguiGlyphRasterizer::glyph`],
    /// if any. A glyph that failed is reported to the layout as missing.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// `ch` at `pixel_size` from the first face that maps it, or `None` when
    /// no face does.
    pub fn render(&mut self, ch: char, pixel_size: u32) -> Result<Option<UguiGlyph>, String> {
        for index in 0..self.faces.len() {
            self.open(index)?;
            let slot = &mut self.faces[index];
            let face = slot.face.as_ref().expect("face opened above");
            if slot.char_size != pixel_size {
                let height = isize::try_from(char_height_26_6(pixel_size))
                    .map_err(|_| format!("pixel size {pixel_size} is out of range"))?;
                face.set_char_size(0, height, 0, DPI)
                    .map_err(|error| format!("set char size {pixel_size}: {error:?}"))?;
                slot.char_size = pixel_size;
            }
            if let Some(glyph_index) = face.get_char_index(ch as usize) {
                return render_glyph(face, glyph_index, ch).map(Some);
            }
        }
        Ok(None)
    }

    /// Opens the face of font `index` if it is not open yet.
    fn open(&mut self, index: usize) -> Result<(), String> {
        if self.faces[index].face.is_some() {
            return Ok(());
        }
        if self.library.is_none() {
            self.library = Some(cff_library()?);
        }
        let library = self.library.as_ref().expect("library initialised above");
        let slot = &mut self.faces[index];
        let bytes = slot
            .bytes
            .take()
            .ok_or_else(|| "open font face: the font failed to open before".to_string())?;
        slot.face = Some(
            library
                .new_memory_face2(bytes, 0)
                .map_err(|error| format!("open font face: {error:?}"))?,
        );
        Ok(())
    }
}

impl<B: Borrow<[u8]>> UguiGlyphRasterizer for UguiFreeTypeFaces<B> {
    fn glyph(&mut self, ch: char, pixel_size: u32) -> Option<Arc<UguiGlyph>> {
        match self.render(ch, pixel_size) {
            Ok(glyph) => glyph.map(Arc::new),
            Err(error) => {
                self.error.get_or_insert(error);
                None
            }
        }
    }
}

fn render_glyph<B>(face: &Face<B>, glyph_index: u32, ch: char) -> Result<UguiGlyph, String> {
    face.load_glyph(glyph_index, LOAD_FLAGS)
        .map_err(|error| format!("load glyph U+{:04X}: {error:?}", u32::from(ch)))?;
    let slot = face.glyph();
    slot.render_glyph(RENDER_MODE)
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
    Ok(UguiGlyph {
        bitmap_left: slot.bitmap_left(),
        bitmap_top: slot.bitmap_top(),
        width: width as u32,
        rows: rows as u32,
        advance_26_6,
        coverage,
    })
}

/// A FreeType library whose CFF driver uses the Adobe engine without stem
/// darkening. Faces keep their own reference to it.
fn cff_library() -> Result<Library, String> {
    let library = Library::init().map_err(|error| format!("init FreeType: {error:?}"))?;
    set_property(&library, b"cff\0", b"hinting-engine\0", &CFF_HINTING_ENGINE)?;
    set_property(
        &library,
        b"cff\0",
        b"no-stem-darkening\0",
        &CFF_NO_STEM_DARKENING,
    )?;
    Ok(library)
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

    const DEJAVU: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";

    fn dejavu() -> Option<Vec<u8>> {
        std::fs::read(DEJAVU).ok()
    }

    #[test]
    fn glyphs_render_unhinted_at_the_requested_pixel_size() {
        let Some(bytes) = dejavu() else {
            eprintln!("{DEJAVU} is not installed; skipping");
            return;
        };
        let mut faces = UguiFreeTypeFaces::new([bytes.as_slice()]);
        let large = faces.glyph('H', 41).expect("H");
        let small = faces.glyph('H', 21).expect("H");
        assert!(large.rows > small.rows && large.width > small.width);
        assert_eq!(large.coverage.len(), (large.width * large.rows) as usize);
        assert!(large.coverage.contains(&255));
        // Unhinted advances keep their fraction of a pixel.
        assert_ne!(large.advance_26_6 % 64, 0);
        // Rendering the same glyph again gives the same bitmap.
        assert_eq!(*faces.glyph('H', 41).expect("H"), *large);
        let space = faces.glyph(' ', 41).expect("space");
        assert!(space.coverage.is_empty() && space.advance_26_6 > 0);
        assert_eq!(faces.glyph('\u{10FFFD}', 41), None);
        assert_eq!(faces.error(), None);
    }

    #[test]
    fn a_character_comes_from_the_first_face_that_maps_it() {
        let Some(bytes) = dejavu() else {
            eprintln!("{DEJAVU} is not installed; skipping");
            return;
        };
        let alone = UguiFreeTypeFaces::new([bytes.as_slice()]).render('H', 30);
        // A face that cannot be opened fails the glyph rather than being
        // skipped.
        let mut broken = UguiFreeTypeFaces::new([b"not a font".as_slice(), bytes.as_slice()]);
        assert!(broken
            .render('H', 30)
            .is_err_and(|error| error.contains("open font face")));
        // With no face mapping the character, it is missing.
        assert_eq!(
            UguiFreeTypeFaces::new([bytes.as_slice(), bytes.as_slice()]).render('\u{10FFFD}', 30),
            Ok(None)
        );
        assert_eq!(
            UguiFreeTypeFaces::new([bytes.as_slice(), bytes.as_slice()]).render('H', 30),
            alone
        );
        assert_eq!(
            UguiFreeTypeFaces::<&[u8]>::new([]).render('H', 30),
            Ok(None)
        );
    }

    #[test]
    fn a_font_file_freetype_cannot_open_is_an_error_not_a_missing_glyph() {
        let mut faces = UguiFreeTypeFaces::new([b"not a font".as_slice()]);
        assert_eq!(faces.glyph('A', 40), None);
        assert!(faces
            .error()
            .is_some_and(|error| error.contains("open font face")));
        // The error is kept; later glyphs stay missing.
        assert_eq!(faces.glyph('B', 40), None);
        assert!(faces
            .error()
            .is_some_and(|error| error.contains("open font face")));
    }

    #[test]
    fn the_character_size_is_the_pixel_size_in_26_6_at_72_dpi() {
        assert_eq!(char_height_26_6(40), 2560);
        assert_eq!(DPI, 72);
        assert_eq!(LOAD_FLAGS, LoadFlag::NO_HINTING);
        assert_eq!(RENDER_MODE as u32, ffi::FT_RENDER_MODE_NORMAL);
    }
}
