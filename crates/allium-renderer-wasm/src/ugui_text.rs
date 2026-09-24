//! uGUI text for the browser renderer.
//!
//! The scene's uGUI text commands are laid out by the shared core layout
//! ([`sekai_profile_renderer_core::ugui_text::layout`]) with glyphs from the
//! shared FreeType faces ([`UguiFreeTypeFaces`]), exactly as the native
//! renderer lays them out. The glyph bitmaps are then packed into 8-bit
//! coverage pages for the WebGL executor, which samples them the way
//! [`sekai_profile_renderer_core::ugui_text::cell_coverage`] does.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use base64::Engine;
use sekai_profile_renderer_core::ugui_freetype::UguiFreeTypeFaces;
use sekai_profile_renderer_core::ugui_text::{
    self, UguiGlyph, UguiGlyphRasterizer, UguiTextSource,
};
use sekai_profile_renderer_core::{Quad, Rect};
use serde::{Deserialize, Serialize};

/// Version of the layout response.
const LAYOUT_VERSION: u32 = 1;
/// Width of a coverage page, in texels.
const PAGE_WIDTH: u32 = 1024;
/// Height a page grows to before glyphs continue on a new page.
const MAX_PAGE_HEIGHT: u32 = 2048;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LayoutRequest {
    /// Font files by family, as byte ranges of the font buffer.
    fonts: Vec<FontRange>,
    texts: Vec<TextRequest>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FontRange {
    family: String,
    offset: usize,
    length: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TextRequest {
    /// Id of the command the text belongs to.
    id: String,
    source: UguiTextSource,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LayoutResponse {
    version: u32,
    texts: Vec<TextLayout>,
    glyphs: Vec<GlyphPlacement>,
    pages: Vec<GlyphPage>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TextLayout {
    id: String,
    /// The text's preferred width in canvas units.
    preferred_width: f32,
    /// Empty texels around the bitmap in every glyph cell.
    cell_padding: u32,
    backdrop: Option<Backdrop>,
    /// Quads that show a glyph, in string order.
    quads: Vec<GlyphQuad>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Backdrop {
    /// The rectangle in layer space, fitted to the preferred width.
    rect: Rect,
    color: [f32; 4],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GlyphQuad {
    /// Node-space corners of the glyph cell: top-left, top-right,
    /// bottom-right, bottom-left.
    corners: Quad,
    /// Index into [`LayoutResponse::glyphs`].
    glyph: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GlyphPlacement {
    page: usize,
    /// Top-left texel of the bitmap in its page.
    x: u32,
    y: u32,
    width: u32,
    rows: u32,
    bitmap_left: i32,
    bitmap_top: i32,
    /// Unhinted advance, 26.6 fixed point.
    advance_26d6: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GlyphPage {
    width: u32,
    height: u32,
    /// `height` rows of `width` coverage bytes, top row first.
    pixels_base64: String,
}

/// Glyphs of one family: its faces and the glyphs rendered so far, so a
/// character repeated across the scene's texts is rendered and packed once.
struct FamilyGlyphs<'a> {
    faces: UguiFreeTypeFaces<&'a [u8]>,
    rendered: HashMap<(u32, char), Option<Arc<UguiGlyph>>>,
}

impl UguiGlyphRasterizer for FamilyGlyphs<'_> {
    fn glyph(&mut self, ch: char, pixel_size: u32) -> Option<Arc<UguiGlyph>> {
        if let Some(glyph) = self.rendered.get(&(pixel_size, ch)) {
            return glyph.clone();
        }
        let glyph = self.faces.glyph(ch, pixel_size);
        // A failed glyph is not kept; the failure fails the layout.
        if self.faces.error().is_none() {
            self.rendered.insert((pixel_size, ch), glyph.clone());
        }
        glyph
    }
}

/// Lays out the texts of `input` with the font files in `fonts` and packs
/// their glyphs. A text whose family has no font file, or whose glyphs
/// FreeType fails to render, fails the whole layout.
pub fn layout_json(fonts: &[u8], input: &str) -> Result<String, String> {
    let request: LayoutRequest = serde_json::from_str(input)
        .map_err(|error| format!("parse uGUI text layout failed: {error}"))?;
    let mut families = BTreeMap::<String, FamilyGlyphs<'_>>::new();
    for range in &request.fonts {
        let bytes = range
            .offset
            .checked_add(range.length)
            .and_then(|end| fonts.get(range.offset..end))
            .ok_or_else(|| format!("font {} is outside the font buffer", range.family))?;
        let previous = families.insert(
            range.family.clone(),
            FamilyGlyphs {
                faces: UguiFreeTypeFaces::new([bytes]),
                rendered: HashMap::new(),
            },
        );
        if previous.is_some() {
            return Err(format!("font {} is given twice", range.family));
        }
    }

    let mut texts = Vec::with_capacity(request.texts.len());
    let mut meshes = Vec::with_capacity(request.texts.len());
    for text in &request.texts {
        let family = &text.source.font.family;
        let glyphs = families
            .get_mut(family)
            .ok_or_else(|| format!("{}: font {family} is not registered", text.id))?;
        let mesh = ugui_text::layout(&text.source, ugui_text::CARD_PIXELS_PER_UNIT, glyphs);
        if let Some(error) = glyphs.faces.error() {
            return Err(format!("{}: glyphs of {family} failed: {error}", text.id));
        }
        texts.push(TextLayout {
            id: text.id.clone(),
            preferred_width: mesh.preferred_width,
            cell_padding: mesh.cell_padding,
            backdrop: text.source.backdrop.as_ref().map(|backdrop| Backdrop {
                rect: backdrop.resolved_rect(mesh.preferred_width),
                color: backdrop.color,
            }),
            quads: Vec::new(),
        });
        meshes.push(mesh);
    }

    // Every distinct glyph bitmap once, in order of first use.
    let mut unique = Vec::<Arc<UguiGlyph>>::new();
    let mut index_of = HashMap::<*const UguiGlyph, usize>::new();
    for (text, mesh) in texts.iter_mut().zip(&meshes) {
        for quad in &mesh.quads {
            let Some(glyph) = &quad.glyph else {
                continue;
            };
            let index = *index_of.entry(Arc::as_ptr(glyph)).or_insert_with(|| {
                unique.push(glyph.clone());
                unique.len() - 1
            });
            text.quads.push(GlyphQuad {
                corners: quad.corners,
                glyph: index,
            });
        }
    }
    let (glyphs, pages) = pack(&unique);
    serde_json::to_string(&LayoutResponse {
        version: LAYOUT_VERSION,
        texts,
        glyphs,
        pages,
    })
    .map_err(|error| error.to_string())
}

/// Packs the bitmaps into pages on shelves, tallest first. Bitmaps are
/// stored without padding: the sampler treats every texel outside a bitmap
/// as empty.
fn pack(glyphs: &[Arc<UguiGlyph>]) -> (Vec<GlyphPlacement>, Vec<GlyphPage>) {
    let mut order = (0..glyphs.len()).collect::<Vec<_>>();
    order.sort_by(|&a, &b| {
        (glyphs[b].rows, glyphs[b].width)
            .cmp(&(glyphs[a].rows, glyphs[a].width))
            .then(a.cmp(&b))
    });
    let mut placements = glyphs
        .iter()
        .map(|glyph| GlyphPlacement {
            page: 0,
            x: 0,
            y: 0,
            width: glyph.width,
            rows: glyph.rows,
            bitmap_left: glyph.bitmap_left,
            bitmap_top: glyph.bitmap_top,
            advance_26d6: glyph.advance_26_6,
        })
        .collect::<Vec<_>>();
    let mut sizes = Vec::<(u32, u32)>::new();
    let (mut x, mut y, mut shelf) = (0u32, 0u32, 0u32);
    for index in order {
        let glyph = &glyphs[index];
        let width = PAGE_WIDTH.max(glyph.width);
        if x + glyph.width > width {
            x = 0;
            y += shelf;
            shelf = 0;
        }
        if sizes.is_empty() || (y + glyph.rows > MAX_PAGE_HEIGHT && y > 0) {
            sizes.push((PAGE_WIDTH, 0));
            x = 0;
            y = 0;
            shelf = 0;
        }
        let page = sizes.len() - 1;
        placements[index].page = page;
        placements[index].x = x;
        placements[index].y = y;
        let size = &mut sizes[page];
        size.0 = size.0.max(x + glyph.width);
        size.1 = size.1.max(y + glyph.rows);
        x += glyph.width;
        shelf = shelf.max(glyph.rows);
    }
    let mut pixels = sizes
        .iter()
        .map(|&(width, height)| vec![0u8; (width * height.max(1)) as usize])
        .collect::<Vec<_>>();
    for (glyph, placement) in glyphs.iter().zip(&placements) {
        let page_width = sizes[placement.page].0 as usize;
        let page = &mut pixels[placement.page];
        for row in 0..glyph.rows as usize {
            let source = &glyph.coverage[row * glyph.width as usize..][..glyph.width as usize];
            let start = (placement.y as usize + row) * page_width + placement.x as usize;
            page[start..start + source.len()].copy_from_slice(source);
        }
    }
    let pages = sizes
        .iter()
        .zip(pixels)
        .map(|(&(width, height), pixels)| GlyphPage {
            width,
            height: height.max(1),
            pixels_base64: base64::engine::general_purpose::STANDARD.encode(pixels),
        })
        .collect();
    (placements, pages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const FIXTURE_FONT: &[u8] = include_bytes!("../scripts/test/fixtures/ugui-fixture.otf");
    const FIXTURE_LAYOUT: &str = "scripts/test/fixtures/ugui-fixture-layout.json";

    fn glyph(width: u32, rows: u32, fill: u8) -> Arc<UguiGlyph> {
        Arc::new(UguiGlyph {
            bitmap_left: 0,
            bitmap_top: rows as i32,
            width,
            rows,
            advance_26_6: 64 * i64::from(width),
            coverage: vec![fill; (width * rows) as usize],
        })
    }

    /// `value` as the response serialises it: `f32` values keep their
    /// shortest decimal form.
    fn json_of(value: &impl Serialize) -> Value {
        serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
    }

    fn page_pixels(page: &GlyphPage) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(&page.pixels_base64)
            .expect("base64 page")
    }

    #[test]
    fn glyphs_pack_on_shelves_without_overlapping_and_keep_their_texels() {
        let glyphs = [glyph(3, 2, 10), glyph(5, 4, 20), glyph(2, 2, 30)];
        let (placements, pages) = pack(&glyphs);
        assert_eq!(pages.len(), 1);
        // Tallest first on one shelf.
        let at = |index: usize| (placements[index].x, placements[index].y);
        assert_eq!([at(1), at(0), at(2)], [(0, 0), (5, 0), (8, 0)]);
        assert_eq!((pages[0].width, pages[0].height), (1024, 4));
        let pixels = page_pixels(&pages[0]);
        assert_eq!(pixels.len(), 1024 * 4);
        assert_eq!(pixels[0], 20);
        assert_eq!(pixels[5], 10);
        assert_eq!(pixels[8], 30);
        // Rows below a shorter bitmap stay empty.
        assert_eq!(pixels[2 * 1024 + 5], 0);
        assert_eq!(pixels[3 * 1024 + 4], 20);
    }

    #[test]
    fn full_shelves_wrap_and_full_pages_continue_on_a_new_page() {
        let wide = (0..3).map(|_| glyph(600, 900, 1)).collect::<Vec<_>>();
        let (placements, pages) = pack(&wide);
        // One per shelf; the third shelf would pass the page height.
        assert_eq!(
            placements
                .iter()
                .map(|placement| (placement.page, placement.y))
                .collect::<Vec<_>>(),
            [(0, 0), (0, 900), (1, 0)]
        );
        assert_eq!(
            pages
                .iter()
                .map(|page| (page.width, page.height))
                .collect::<Vec<_>>(),
            [(1024, 1800), (1024, 900)]
        );
        assert_eq!(pack(&[]).1.len(), 0);
    }

    fn fixture_request(texts: Value) -> String {
        serde_json::json!({
            "fonts": [{ "family": "UguiFixture", "offset": 0, "length": FIXTURE_FONT.len() }],
            "texts": texts,
        })
        .to_string()
    }

    fn fixture() -> Value {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/test/fixtures/ugui-fixture-layout.json"
        );
        serde_json::from_str(&std::fs::read_to_string(path).expect(FIXTURE_LAYOUT))
            .expect("fixture layout JSON")
    }

    /// The layout response with every page's pixels replaced by their
    /// SHA-256, as the fixture stores it.
    fn hashed(response: &str) -> Value {
        use sha2::Digest as _;
        let mut value: Value = serde_json::from_str(response).expect("layout JSON");
        for page in value["pages"].as_array_mut().expect("pages") {
            let pixels = base64::engine::general_purpose::STANDARD
                .decode(page["pixelsBase64"].as_str().expect("page pixels"))
                .expect("base64");
            let object = page.as_object_mut().expect("page");
            object.remove("pixelsBase64");
            object.insert(
                "pixelsSha256".into(),
                Value::String(format!("{:x}", sha2::Sha256::digest(&pixels))),
            );
        }
        value
    }

    #[test]
    fn the_fixture_font_lays_out_as_the_recorded_native_layout() {
        let fixture = fixture();
        let response = layout_json(
            FIXTURE_FONT,
            &fixture_request(fixture["request"]["texts"].clone()),
        )
        .expect("layout");
        let actual = hashed(&response);
        if std::env::var_os("SEKAI_PROFILE_UPDATE_FIXTURES").is_some() {
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/scripts/test/fixtures/ugui-fixture-layout.json"
            );
            let updated = serde_json::json!({ "request": fixture["request"], "layout": actual });
            std::fs::write(path, serde_json::to_string_pretty(&updated).unwrap() + "\n").unwrap();
            return;
        }
        assert_eq!(actual, fixture["layout"]);
    }

    #[test]
    fn the_layout_is_the_core_layout_with_the_shared_freetype_faces() {
        let fixture = fixture();
        let texts = fixture["request"]["texts"].as_array().expect("texts");
        let response: Value = serde_json::from_str(
            &layout_json(FIXTURE_FONT, &fixture_request(Value::Array(texts.clone())))
                .expect("layout"),
        )
        .unwrap();
        let mut faces = UguiFreeTypeFaces::new([FIXTURE_FONT]);
        for (text, laid_out) in texts.iter().zip(response["texts"].as_array().unwrap()) {
            let source: UguiTextSource = serde_json::from_value(text["source"].clone()).unwrap();
            let mesh = ugui_text::layout(&source, ugui_text::CARD_PIXELS_PER_UNIT, &mut faces);
            let drawn = mesh
                .quads
                .iter()
                .filter(|quad| quad.glyph.is_some())
                .map(|quad| json_of(&quad.corners))
                .collect::<Vec<_>>();
            let quads = laid_out["quads"]
                .as_array()
                .unwrap()
                .iter()
                .map(|quad| quad["corners"].clone())
                .collect::<Vec<_>>();
            assert_eq!(quads, drawn);
            assert_eq!(laid_out["preferredWidth"], json_of(&mesh.preferred_width));
            for (quad, glyph) in laid_out["quads"]
                .as_array()
                .unwrap()
                .iter()
                .zip(mesh.quads.iter().filter_map(|quad| quad.glyph.as_ref()))
            {
                let placement = &response["glyphs"][quad["glyph"].as_u64().unwrap() as usize];
                assert_eq!(placement["width"], glyph.width);
                assert_eq!(placement["rows"], glyph.rows);
                assert_eq!(placement["bitmapLeft"], glyph.bitmap_left);
                assert_eq!(placement["bitmapTop"], glyph.bitmap_top);
                assert_eq!(placement["advance26d6"], glyph.advance_26_6);
                // The page holds the bitmap at its placement.
                let page = &response["pages"][placement["page"].as_u64().unwrap() as usize];
                let pixels = base64::engine::general_purpose::STANDARD
                    .decode(page["pixelsBase64"].as_str().unwrap())
                    .unwrap();
                let page_width = page["width"].as_u64().unwrap() as usize;
                let (x, y) = (
                    placement["x"].as_u64().unwrap() as usize,
                    placement["y"].as_u64().unwrap() as usize,
                );
                for row in 0..glyph.rows as usize {
                    let start = (y + row) * page_width + x;
                    assert_eq!(
                        &pixels[start..start + glyph.width as usize],
                        &glyph.coverage[row * glyph.width as usize..][..glyph.width as usize]
                    );
                }
            }
        }
        assert_eq!(faces.error(), None);
    }

    #[test]
    fn a_text_without_its_font_or_with_a_broken_font_fails_the_layout() {
        let text = |family: &str| {
            let mut source = fixture()["request"]["texts"][0].clone();
            source["source"]["font"]["family"] = Value::String(family.into());
            Value::Array(vec![source])
        };
        let missing = layout_json(FIXTURE_FONT, &fixture_request(text("OtherFamily")));
        assert!(
            missing
                .as_ref()
                .is_err_and(|error| error.contains("font OtherFamily is not registered")),
            "{missing:?}"
        );
        let broken = layout_json(
            b"not a font",
            &serde_json::json!({
                "fonts": [{ "family": "UguiFixture", "offset": 0, "length": 10 }],
                "texts": text("UguiFixture"),
            })
            .to_string(),
        );
        assert!(
            broken
                .as_ref()
                .is_err_and(|error| error.contains("open font face")),
            "{broken:?}"
        );
        let outside = layout_json(
            b"short",
            &serde_json::json!({
                "fonts": [{ "family": "UguiFixture", "offset": 2, "length": 10 }],
                "texts": [],
            })
            .to_string(),
        );
        assert!(outside.is_err_and(|error| error.contains("outside the font buffer")));
    }

    #[test]
    fn repeated_characters_share_one_packed_glyph_and_backdrops_fit_the_text() {
        let fixture = fixture();
        let mut first = fixture["request"]["texts"][0].clone();
        first["source"]["text"] = Value::String("AOA".into());
        first["source"]["backdrop"] = serde_json::json!({
            "color": [0.0, 0.8, 0.733_333_35, 1.0],
            "rect": { "x": -6.0, "y": -210.2, "width": 50.0, "height": 126.0 },
            "fit_padding": 34.0
        });
        let mut second = first.clone();
        second["id"] = Value::String("second".into());
        second["source"]["backdrop"] = Value::Null;
        let response: Value = serde_json::from_str(
            &layout_json(
                FIXTURE_FONT,
                &fixture_request(serde_json::json!([first, second])),
            )
            .expect("layout"),
        )
        .unwrap();
        let glyph_indices = |text: usize| {
            response["texts"][text]["quads"]
                .as_array()
                .unwrap()
                .iter()
                .map(|quad| quad["glyph"].as_u64().unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(glyph_indices(0), [0, 1, 0]);
        assert_eq!(glyph_indices(1), [0, 1, 0]);
        assert_eq!(response["glyphs"].as_array().unwrap().len(), 2);
        let preferred = response["texts"][0]["preferredWidth"].as_f64().unwrap() as f32;
        assert_eq!(
            response["texts"][0]["backdrop"]["rect"]["height"]
                .as_f64()
                .unwrap() as f32,
            preferred + 34.0
        );
        assert_eq!(
            response["texts"][0]["backdrop"]["rect"]["y"]
                .as_f64()
                .unwrap() as f32,
            -210.2
        );
        assert!(response["texts"][1]["backdrop"].is_null());
    }
}
