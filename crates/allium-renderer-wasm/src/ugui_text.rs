//! uGUI text for the browser renderer.
//!
//! The scene's uGUI text commands are laid out by the shared core layout
//! ([`sekai_profile_renderer_core::ugui_text::layout`]) with glyphs from the
//! shared FreeType faces ([`UguiFreeTypeFaces`]) of each text's face chain,
//! exactly as the native renderer lays them out. Every family of a chain
//! needs its font file. The glyph bitmaps are then packed into 8-bit
//! coverage pages for the WebGL executor, which samples them the way
//! [`sekai_profile_renderer_core::ugui_text::cell_coverage`] does.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use base64::Engine;
use sekai_profile_renderer_core::ugui_freetype::UguiFreeTypeFaces;
use sekai_profile_renderer_core::ugui_text::{
    self, UguiFontFace, UguiGlyph, UguiGlyphRasterizer, UguiTextSource,
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

/// Glyphs of one face chain: its faces and the glyphs rendered so far, so a
/// character repeated across the texts of the chain is rendered and packed
/// once.
struct ChainGlyphs<'a> {
    faces: UguiFreeTypeFaces<&'a [u8]>,
    rendered: HashMap<(u32, char), Option<Arc<UguiGlyph>>>,
}

impl UguiGlyphRasterizer for ChainGlyphs<'_> {
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
/// their glyphs. A text with a family of its face chain that has no font
/// file, or whose glyphs FreeType fails to render, fails the whole layout.
pub fn layout_json(fonts: &[u8], input: &str) -> Result<String, String> {
    let request: LayoutRequest = serde_json::from_str(input)
        .map_err(|error| format!("parse uGUI text layout failed: {error}"))?;
    let mut files = BTreeMap::<&str, &[u8]>::new();
    for range in &request.fonts {
        let bytes = range
            .offset
            .checked_add(range.length)
            .and_then(|end| fonts.get(range.offset..end))
            .ok_or_else(|| format!("font {} is outside the font buffer", range.family))?;
        if files.insert(&range.family, bytes).is_some() {
            return Err(format!("font {} is given twice", range.family));
        }
    }

    let mut chains = BTreeMap::<Vec<UguiFontFace>, ChainGlyphs<'_>>::new();
    let mut texts = Vec::with_capacity(request.texts.len());
    let mut meshes = Vec::with_capacity(request.texts.len());
    for text in &request.texts {
        let chain = text.source.face_chain();
        let families = chain
            .iter()
            .map(|face| face.family.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let glyphs = match chains.entry(chain) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let faces = entry
                    .key()
                    .iter()
                    .map(|face| {
                        files
                            .get(face.family.as_str())
                            .map(|bytes| (*bytes, face.face_index))
                            .ok_or_else(|| {
                                format!("{}: font {} is not registered", text.id, face.family)
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                entry.insert(ChainGlyphs {
                    faces: UguiFreeTypeFaces::new(faces),
                    rendered: HashMap::new(),
                })
            }
        };
        let mesh = ugui_text::layout(&text.source, ugui_text::CARD_PIXELS_PER_UNIT, glyphs);
        if let Some(error) = glyphs.faces.error() {
            return Err(format!("{}: glyphs of {families} failed: {error}", text.id));
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
    /// The synthetic fixture fonts by file name.
    const FIXTURE_FILES: [(&str, &[u8]); 3] = [
        ("ugui-fixture.otf", FIXTURE_FONT),
        (
            "ugui-fixture-latin.ttf",
            include_bytes!("../scripts/test/fixtures/ugui-fixture-latin.ttf"),
        ),
        (
            "ugui-fixture-cjk.ttc",
            include_bytes!("../scripts/test/fixtures/ugui-fixture-cjk.ttc"),
        ),
    ];
    /// Recorded layouts: the text's own font alone, and fallback faces.
    const FIXTURE_LAYOUTS: [&str; 2] = [
        "ugui-fixture-layout.json",
        "ugui-fixture-fallback-layout.json",
    ];

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

    fn fixture_path(name: &str) -> String {
        format!(
            "{}/scripts/test/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    fn fixture_named(name: &str) -> Value {
        let path = fixture_path(name);
        serde_json::from_str(&std::fs::read_to_string(&path).expect(&path))
            .expect("fixture layout JSON")
    }

    fn fixture() -> Value {
        fixture_named(FIXTURE_LAYOUTS[0])
    }

    fn fixture_file(name: &str) -> &'static [u8] {
        FIXTURE_FILES
            .iter()
            .find_map(|(file, bytes)| (*file == name).then_some(*bytes))
            .unwrap_or_else(|| panic!("no fixture font {name}"))
    }

    /// The font buffer and layout request of a recorded layout: its font
    /// files one after another, then its texts.
    fn recorded_request(fixture: &Value) -> (Vec<u8>, String) {
        let mut buffer = Vec::new();
        let mut fonts = Vec::new();
        for font in fixture["request"]["fonts"].as_array().expect("fonts") {
            let bytes = fixture_file(font["file"].as_str().expect("file"));
            fonts.push(serde_json::json!({
                "family": font["family"],
                "offset": buffer.len(),
                "length": bytes.len(),
            }));
            buffer.extend_from_slice(bytes);
        }
        let request = serde_json::json!({ "fonts": fonts, "texts": fixture["request"]["texts"] });
        (buffer, request.to_string())
    }

    /// The faces of `source`'s chain from the recorded layout's font files.
    fn recorded_faces(
        fixture: &Value,
        source: &UguiTextSource,
    ) -> UguiFreeTypeFaces<&'static [u8]> {
        let fonts = fixture["request"]["fonts"].as_array().expect("fonts");
        UguiFreeTypeFaces::new(source.face_chain().into_iter().map(|face| {
            let font = fonts
                .iter()
                .find(|font| font["family"] == face.family.as_str())
                .unwrap_or_else(|| panic!("no fixture font for {}", face.family));
            (
                fixture_file(font["file"].as_str().expect("file")),
                face.face_index,
            )
        }))
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
    fn the_fixture_fonts_lay_out_as_the_recorded_native_layouts() {
        for name in FIXTURE_LAYOUTS {
            let fixture = fixture_named(name);
            let (fonts, request) = recorded_request(&fixture);
            let actual = hashed(&layout_json(&fonts, &request).expect("layout"));
            if std::env::var_os("SEKAI_PROFILE_UPDATE_FIXTURES").is_some() {
                let updated =
                    serde_json::json!({ "request": fixture["request"], "layout": actual });
                std::fs::write(
                    fixture_path(name),
                    serde_json::to_string_pretty(&updated).unwrap() + "\n",
                )
                .unwrap();
                continue;
            }
            assert_eq!(actual, fixture["layout"], "{name}");
        }
    }

    #[test]
    fn the_layout_is_the_core_layout_with_the_shared_freetype_faces() {
        for name in FIXTURE_LAYOUTS {
            assert_core_layout(&fixture_named(name));
        }
    }

    fn assert_core_layout(fixture: &Value) {
        let texts = fixture["request"]["texts"].as_array().expect("texts");
        let (fonts, request) = recorded_request(fixture);
        let response: Value =
            serde_json::from_str(&layout_json(&fonts, &request).expect("layout")).unwrap();
        for (text, laid_out) in texts.iter().zip(response["texts"].as_array().unwrap()) {
            let source: UguiTextSource = serde_json::from_value(text["source"].clone()).unwrap();
            let mut faces = recorded_faces(fixture, &source);
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
            assert_eq!(faces.error(), None);
        }
    }

    /// The text `id` of the fallback fixture, its layout and its glyphs'
    /// placements.
    fn fallback_text(id: &str) -> (UguiTextSource, Value, Vec<Value>) {
        let fixture = fixture_named(FIXTURE_LAYOUTS[1]);
        let (fonts, request) = recorded_request(&fixture);
        let response: Value =
            serde_json::from_str(&layout_json(&fonts, &request).expect("layout")).unwrap();
        let index = fixture["request"]["texts"]
            .as_array()
            .unwrap()
            .iter()
            .position(|text| text["id"] == id)
            .unwrap_or_else(|| panic!("no fallback text {id}"));
        let source =
            serde_json::from_value(fixture["request"]["texts"][index]["source"].clone()).unwrap();
        let laid_out = response["texts"][index].clone();
        let placements = laid_out["quads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|quad| response["glyphs"][quad["glyph"].as_u64().unwrap() as usize].clone())
            .collect();
        (source, laid_out, placements)
    }

    /// Bitmap offsets, size and advance of `ch` in face `face_index` of a
    /// fixture file at `pixel_size`.
    fn face_metrics(file: &str, face_index: u32, ch: char, pixel_size: u32) -> Value {
        let glyph = UguiFreeTypeFaces::new([(fixture_file(file), face_index)])
            .render(ch, pixel_size)
            .expect("render")
            .unwrap_or_else(|| panic!("{file} face {face_index} has no {ch}"));
        serde_json::json!({
            "bitmapLeft": glyph.bitmap_left,
            "bitmapTop": glyph.bitmap_top,
            "width": glyph.width,
            "rows": glyph.rows,
            "advance26d6": glyph.advance_26_6,
        })
    }

    fn metrics_of(placement: &Value) -> Value {
        serde_json::json!({
            "bitmapLeft": placement["bitmapLeft"],
            "bitmapTop": placement["bitmapTop"],
            "width": placement["width"],
            "rows": placement["rows"],
            "advance26d6": placement["advance26d6"],
        })
    }

    #[test]
    fn each_character_comes_from_the_first_face_of_its_chain_that_maps_it() {
        // "AB关Z": the text's font maps only A; the Latin face maps B (the
        // collection maps it too, later); the collection's face 2 maps 关;
        // no face maps Z, which keeps an empty cell.
        let (source, laid_out, placements) = fallback_text("chain");
        assert_eq!(source.text, "AB关Z");
        assert_eq!(placements.len(), 3);
        assert_eq!(
            placements.iter().map(metrics_of).collect::<Vec<_>>(),
            [
                face_metrics("ugui-fixture.otf", 0, 'A', 40),
                face_metrics("ugui-fixture-latin.ttf", 0, 'B', 40),
                face_metrics("ugui-fixture-cjk.ttc", 2, '关', 40),
            ]
        );
        assert_ne!(
            face_metrics("ugui-fixture-cjk.ttc", 2, 'B', 40),
            metrics_of(&placements[1])
        );
        assert_ne!(
            face_metrics("ugui-fixture-latin.ttf", 0, 'A', 40),
            metrics_of(&placements[0])
        );
        // Without fallback faces B and 关 are missing too.
        let (_, alone, _) = fallback_text("no-fallback");
        assert_eq!(alone["quads"].as_array().unwrap().len(), 1);
        assert!(laid_out["preferredWidth"].as_f64() > alone["preferredWidth"].as_f64());
    }

    #[test]
    fn the_face_index_picks_the_face_of_the_collection() {
        let metrics = (0..4)
            .map(|face_index| {
                let (source, _, placements) = fallback_text(&format!("face-{face_index}"));
                assert_eq!(source.fallback_faces[1].face_index, face_index);
                let metrics = metrics_of(&placements[0]);
                assert_eq!(
                    metrics,
                    face_metrics("ugui-fixture-cjk.ttc", face_index, '关', 30)
                );
                metrics
            })
            .collect::<Vec<_>>();
        // The four faces draw 关 differently.
        for (index, a) in metrics.iter().enumerate() {
            for b in &metrics[index + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn a_fallback_glyph_keeps_the_font_assets_padding_and_advance_rounding() {
        let (source, laid_out, placements) = fallback_text("chain");
        let padding = f64::from(source.font.character_padding);
        assert_eq!(laid_out["cellPadding"], source.font.character_padding);
        assert!(source.font.round_advance && source.font.tracking == 1.0);
        let quads = laid_out["quads"].as_array().unwrap();
        let left = |index: usize| quads[index]["corners"][0][0].as_f64().unwrap();
        let right = |index: usize| quads[index]["corners"][1][0].as_f64().unwrap();
        let number = |placement: &Value, key: &str| placement[key].as_f64().unwrap();
        // Pen positions step by the rounded advances of the glyphs' own
        // faces.
        let advance = |placement: &Value| (number(placement, "advance26d6") / 64.0 + 0.5).floor();
        let latin_advance = number(&placements[1], "advance26d6") / 64.0;
        assert_ne!(
            latin_advance.fract(),
            0.0,
            "the Latin advance is fractional"
        );
        let mut pen = 0.0;
        for (index, placement) in placements.iter().enumerate() {
            // Each cell is the bitmap plus the asset's padding on both sides.
            assert_eq!(
                left(index),
                pen + number(placement, "bitmapLeft") - padding,
                "glyph {index}"
            );
            assert_eq!(
                right(index) - left(index),
                number(placement, "width") + 2.0 * padding,
                "glyph {index}"
            );
            pen += advance(placement);
        }
        // Z adds nothing to the preferred width.
        assert_eq!(laid_out["preferredWidth"].as_f64().unwrap(), pen);
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
        // Every fallback family of the chain needs its file too, whether or
        // not a character needs it.
        let mut fallback = text("UguiFixture");
        fallback[0]["source"]["fallback_faces"] =
            serde_json::json!([{ "family": "Missing Fallback", "face_index": 0 }]);
        let missing = layout_json(FIXTURE_FONT, &fixture_request(fallback));
        assert!(
            missing.as_ref().is_err_and(
                |error| error.contains("title: font Missing Fallback is not registered")
            ),
            "{missing:?}"
        );
        // A face the collection does not have fails the layout.
        let fixture = fixture_named(FIXTURE_LAYOUTS[1]);
        let mut beyond = fixture.clone();
        let texts = beyond["request"]["texts"].as_array_mut().unwrap();
        texts.truncate(1);
        texts[0]["source"]["fallback_faces"][1]["face_index"] = serde_json::json!(4);
        let (fonts, request) = recorded_request(&beyond);
        let beyond = layout_json(&fonts, &request);
        assert!(
            beyond.as_ref().is_err_and(|error| error.contains(
                "glyphs of UguiFixture, UguiFixture Latin, UguiFixture CJK failed: open font face 4 of font 2"
            )),
            "{beyond:?}"
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
