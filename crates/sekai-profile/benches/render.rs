//! Render and encode benchmarks over synthetic inputs.
//!
//! Every input is built here: a text atlas generated from DejaVu Sans, a
//! two-row master data set naming that face, and cards of plain text. No
//! game font, asset or master data is read. Point `FONT_DIR` (or
//! `SEKAI_PROFILE_FONT_DIR`) at a directory holding `DejaVuSans.ttf`, as the
//! CI workflow does, and run
//!
//! ```sh
//! cargo bench -p sekai-profile --features jpeg-turbo --bench render
//! ```
//!
//! Without the font the benchmarks that need it are skipped. The SIMD and
//! AVX-512 benchmarks run only on a CPU that supports them.

use std::collections::BTreeSet;
use std::hint::black_box;
use std::sync::Arc;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use sekai_profile::prepare::{build_text_atlas, TextAtlasOptions};
use sekai_profile::sdf::atlas::MappedSdfAtlas;
use sekai_profile::sdf::outline::{OfflineAtlasGlyphGenerator, OfflineGenerationMethod};
use sekai_profile::sdf::tile::{
    Affine2, Point2, SdfAccumulationMode, SdfDrawCommand, SdfMaterial, SdfPrimitiveKind,
    SdfTilePlan, TileGrid,
};
use sekai_profile::types::CustomProfileCard;
use sekai_profile::{
    CustomProfileRenderer, JsonMasterDataProvider, RenderConfig, SdfExecutor,
};

/// The size of a rendered custom profile page.
const PAGE_WIDTH: u32 = 1830;
const PAGE_HEIGHT: u32 = 812;
const FAMILY: &str = "DejaVu Sans";
const LINE: &str = "The quick brown fox jumps over the lazy dog 0123456789";

/// A text atlas of DejaVu Sans covering [`LINE`], built once per process.
struct Fixture {
    _dir: tempfile::TempDir,
    atlas: Arc<MappedSdfAtlas>,
}

fn fixture() -> Option<&'static Fixture> {
    static FIXTURE: std::sync::OnceLock<Option<Fixture>> = std::sync::OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            if sekai_profile::sdf::outline::resolve_font_path(FAMILY).is_none() {
                eprintln!("{FAMILY} is not installed; set FONT_DIR to run these benchmarks");
                return None;
            }
            let dir = tempfile::tempdir().expect("atlas directory");
            let mut options = TextAtlasOptions::new(FAMILY, dir.path().join("text"));
            options.method = OfflineGenerationMethod::Analytic;
            options.page_size = 512;
            options.codepoints = Some(LINE.chars().map(u32::from).collect::<BTreeSet<_>>());
            let manifest = build_text_atlas(&options).expect("build the text atlas");
            let atlas = MappedSdfAtlas::open(&manifest).expect("open the text atlas");
            Some(Fixture {
                _dir: dir,
                atlas: Arc::new(atlas),
            })
        })
        .as_ref()
}

/// Lines of [`LINE`] filling a page at `font_size`, one draw command per
/// glyph.
fn page_commands(atlas: &MappedSdfAtlas, font_size: f32) -> Vec<SdfDrawCommand> {
    let manifest = atlas.manifest();
    let scale = font_size / manifest.point_size;
    let line_height = font_size * 1.3;
    let mut commands = Vec::new();
    let mut baseline = font_size;
    while baseline < PAGE_HEIGHT as f32 {
        let mut x = 16.0;
        while x < PAGE_WIDTH as f32 - font_size {
            for ch in LINE.chars() {
                let Some(glyph) = atlas.glyph(u32::from(ch)) else {
                    continue;
                };
                if let Ok(command) = SdfDrawCommand::from_atlas_glyph(
                    SdfPrimitiveKind::Text,
                    0,
                    glyph,
                    manifest.point_size,
                    manifest.spread,
                    Point2::new(x, baseline),
                    font_size,
                    Affine2::IDENTITY,
                    SdfMaterial::default(),
                ) {
                    commands.push(command);
                }
                x += glyph.plane_advance_x * scale;
                if x >= PAGE_WIDTH as f32 - font_size {
                    break;
                }
            }
        }
        baseline += line_height;
    }
    commands
}

fn sdf_executors(c: &mut Criterion) {
    let Some(fixture) = fixture() else { return };
    let atlas = fixture.atlas.as_ref();
    let grid = TileGrid::new(PAGE_WIDTH, PAGE_HEIGHT);
    let pixels = u64::from(PAGE_WIDTH) * u64::from(PAGE_HEIGHT);
    let mut group = c.benchmark_group("sdf");
    group.throughput(Throughput::Elements(pixels));
    for font_size in [16.0f32, 48.0] {
        let commands = page_commands(atlas, font_size);
        let id = format!("{}px", font_size as u32);
        group.bench_with_input(BenchmarkId::new("plan", &id), &commands, |b, commands| {
            b.iter(|| SdfTilePlan::build(grid, black_box(commands), atlas).expect("plan"))
        });
        let plan = SdfTilePlan::build(grid, &commands, atlas).expect("plan");
        let mut output = vec![0u8; pixels as usize * 4];
        group.bench_with_input(BenchmarkId::new("scalar", &id), &plan, |b, plan| {
            b.iter(|| {
                plan.execute_scalar_f32(atlas, [255; 4], black_box(&mut output))
                    .expect("scalar")
            })
        });
        if sekai_profile::sdf_simd_available() {
            group.bench_with_input(BenchmarkId::new("simd", &id), &plan, |b, plan| {
                b.iter(|| {
                    plan.execute_simd(
                        atlas,
                        [255; 4],
                        black_box(&mut output),
                        SdfAccumulationMode::F32Tile,
                    )
                    .expect("simd")
                })
            });
        }
    }
    group.finish();
}

fn glyph_generation(c: &mut Criterion) {
    if fixture().is_none() {
        return;
    }
    let generator = OfflineAtlasGlyphGenerator::new(FAMILY).expect("glyph generator");
    let mut group = c.benchmark_group("glyph");
    for (name, method) in [
        ("analytic", OfflineGenerationMethod::Analytic),
        ("edt-2x", OfflineGenerationMethod::Edt { supersample: 2 }),
    ] {
        group.bench_function(name, |b| {
            b.iter(|| {
                for ch in ['A', 'g', '8'] {
                    black_box(generator.generate(ch, method).expect("glyph"));
                }
            })
        });
    }
    group.finish();
}

/// A page drawn by the SDF executor, as the encoders receive it.
fn rendered_pixels() -> Vec<u8> {
    let mut output = vec![255u8; PAGE_WIDTH as usize * PAGE_HEIGHT as usize * 4];
    if let Some(fixture) = fixture() {
        let atlas = fixture.atlas.as_ref();
        let commands = page_commands(atlas, 24.0);
        let plan = SdfTilePlan::build(TileGrid::new(PAGE_WIDTH, PAGE_HEIGHT), &commands, atlas)
            .expect("plan");
        plan.execute_scalar_f32(atlas, [255; 4], &mut output)
            .expect("scalar");
    }
    output
}

fn encoders(c: &mut Criterion) {
    let rgba = rendered_pixels();
    let mut group = c.benchmark_group("encode");
    group.throughput(Throughput::Elements(
        u64::from(PAGE_WIDTH) * u64::from(PAGE_HEIGHT),
    ));
    group.sample_size(20);
    group.bench_function("png", |b| {
        b.iter(|| {
            sekai_profile::codec::png::encode_rgba(PAGE_WIDTH, PAGE_HEIGHT, black_box(&rgba))
                .expect("png")
        })
    });
    #[cfg(feature = "jpeg-turbo")]
    {
        group.bench_function("jpeg-standard-q90", |b| {
            b.iter(|| {
                sekai_profile::jpeg_turbo::encode_rgba(
                    black_box(&rgba),
                    PAGE_WIDTH,
                    PAGE_HEIGHT,
                    90,
                )
                .expect("jpeg")
            })
        });
        if sekai_profile::sdf_simd_available() {
            let mut scratch = Vec::new();
            group.bench_function("jpeg-avx512-yuv420-q90", |b| {
                b.iter(|| {
                    sekai_profile::jpeg_turbo::encode_rgba_avx512_yuv420_with_scratch(
                        black_box(&rgba),
                        PAGE_WIDTH,
                        PAGE_HEIGHT,
                        90,
                        &mut scratch,
                    )
                    .expect("jpeg")
                })
            });
        }
    }
    group.finish();
}

/// A renderer whose master data names [`FAMILY`] for font 1, with the
/// fixture atlas installed.
fn page_renderer(fixture: &Fixture) -> CustomProfileRenderer {
    let mut masterdata = JsonMasterDataProvider::empty();
    masterdata
        .insert_table(
            "customProfileTextFonts",
            &format!(r#"[{{"id": 1, "fontName": "{FAMILY}"}}]"#),
        )
        .expect("font table");
    masterdata
        .insert_table(
            "customProfileTextColors",
            r##"[{"id": 1, "colorCode": "#202020"}]"##,
        )
        .expect("color table");
    CustomProfileRenderer::new(Arc::new(masterdata))
        .with_sdf_atlas(Arc::clone(&fixture.atlas))
        .expect("install the atlas")
}

/// A card of `lines` text elements stacked down the page.
fn text_card(lines: usize) -> CustomProfileCard {
    let texts: Vec<_> = (0..lines)
        .map(|index| {
            serde_json::json!({
                "objectData": {
                    "layer": index, "lock": false, "visible": true,
                    "position": { "x": 0.0, "y": 360.0 - index as f32 * 48.0, "z": 0.0 },
                    "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                    "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
                },
                "colorId": 1, "fontId": 1, "lineSpacing": 0.0,
                "outlineColorId": 1, "outlineSize": 0.0, "size": 32.0,
                "text": LINE, "type": 513
            })
        })
        .collect();
    serde_json::from_value(serde_json::json!({ "texts": texts })).expect("text card")
}

fn pages(c: &mut Criterion) {
    let Some(fixture) = fixture() else { return };
    let renderer = page_renderer(fixture);
    let card = text_card(16);
    let mut executors = vec![("scalar", SdfExecutor::Scalar)];
    if sekai_profile::sdf_simd_available() {
        executors.push(("simd", SdfExecutor::Simd));
    }
    let mut group = c.benchmark_group("page");
    group.sample_size(20);
    for (name, executor) in executors {
        let config = RenderConfig {
            executor,
            ..RenderConfig::default()
        };
        group.bench_function(BenchmarkId::new("render", name), |b| {
            b.iter(|| renderer.render(black_box(&card), None, &config).expect("page"))
        });
        let batch: Vec<_> = (1..=3).map(|seq| (seq, &card)).collect();
        group.bench_function(BenchmarkId::new("render-batch-3", name), |b| {
            b.iter(|| {
                renderer
                    .render_batch("bench", "cn", batch.iter().copied(), None, &config)
                    .expect("batch")
            })
        });
    }
    group.finish();
}

fn configure() -> Criterion {
    Criterion::default().configure_from_args()
}

criterion_group! {
    name = benches;
    config = configure();
    targets = sdf_executors, glyph_generation, encoders, pages
}
criterion_main!(benches);
