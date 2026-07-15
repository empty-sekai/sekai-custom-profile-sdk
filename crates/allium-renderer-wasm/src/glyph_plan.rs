use freetype::{face::LoadFlag, Library};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::rc::Rc;

use super::{atlas::AtlasConfig, FONT_ENGINE_FINGERPRINT, TMP_POINT_SIZE, TMP_SPREAD};

const CACHE_SCHEMA: &str = "allium.glyph-raster-cache.v1";
const THRESHOLD: u32 = 128;
const DOWNSAMPLE_VERSION: &str = "box-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RasterBackend {
    Edt,
    Analytic,
}

impl RasterBackend {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "edt" => Ok(Self::Edt),
            "analytic" => Ok(Self::Analytic),
            _ => Err(format!("unsupported SDF backend {value}")),
        }
    }

    fn load_flags(self) -> &'static str {
        match self {
            Self::Edt => "NO_HINTING",
            Self::Analytic => "NO_BITMAP|NO_HINTING",
        }
    }

    fn render_mode(self) -> &'static str {
        match self {
            Self::Edt => "normal-mask",
            Self::Analytic => "outline",
        }
    }

    fn normalized_supersample(self, requested: usize) -> usize {
        match self {
            Self::Edt => requested.clamp(1, 4),
            Self::Analytic => 0,
        }
    }

    fn load_flag(self) -> LoadFlag {
        match self {
            Self::Edt => LoadFlag::NO_HINTING,
            Self::Analytic => LoadFlag::NO_BITMAP | LoadFlag::NO_HINTING,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Edt => "edt",
            Self::Analytic => "analytic",
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlyphRasterPlan {
    region: String,
    family: String,
    font_source_hash: String,
    schema_namespace: &'static str,
    font_engine_fingerprint: &'static str,
    raster_contract_id: String,
    contract_id: String,
    backend: RasterBackend,
    supersample: usize,
    base_size: f32,
    spread: f32,
    atlas_width: usize,
    atlas_height: usize,
    glyphs: Vec<PlannedGlyph>,
    missing: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlannedGlyph {
    ch: String,
    glyph_index: u32,
    identity: GlyphRasterIdentity,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GlyphRasterIdentity {
    opaque_key: String,
    schema_namespace: &'static str,
    font_engine_fingerprint: &'static str,
    raster_contract_id: String,
}

pub fn plan(
    font_bytes: Vec<u8>,
    codepoints: &[u32],
    region: String,
    family: String,
    font_source_hash: String,
    backend: RasterBackend,
    requested_supersample: usize,
) -> Result<GlyphRasterPlan, String> {
    let supersample = backend.normalized_supersample(requested_supersample);
    let raster_contract_id = format!(
        "allium-r8-{}-ss{supersample}-spread{}-threshold{THRESHOLD}-{DOWNSAMPLE_VERSION}",
        backend.name(),
        TMP_SPREAD as usize,
    );
    let contract_id = format!("{FONT_ENGINE_FINGERPRINT}:{raster_contract_id}");
    let library = Library::init().map_err(|error| format!("FreeType init failed: {error:?}"))?;
    let face = library
        .new_memory_face(Rc::new(font_bytes), 0)
        .map_err(|error| format!("load memory face failed: {error:?}"))?;
    face.set_char_size((TMP_POINT_SIZE as isize) * 64, 0, 72, 72)
        .map_err(|error| format!("set char size failed: {error:?}"))?;
    let mut glyphs = Vec::with_capacity(codepoints.len());
    let mut missing = Vec::new();
    for codepoint in codepoints {
        let Some(ch) = char::from_u32(*codepoint) else {
            missing.push(format!("U+{codepoint:04X}"));
            continue;
        };
        let Some(glyph_index) = face.get_char_index(ch as usize) else {
            missing.push(ch.to_string());
            continue;
        };
        face.load_glyph(glyph_index, backend.load_flag())
            .map_err(|error| format!("load glyph plan failed: {error:?}"))?;
        glyphs.push(PlannedGlyph {
            ch: ch.to_string(),
            glyph_index,
            identity: identity(
                &region,
                &font_source_hash,
                glyph_index,
                backend,
                supersample,
                &raster_contract_id,
            )?,
        });
    }
    let atlas = AtlasConfig::default();
    Ok(GlyphRasterPlan {
        region,
        family,
        font_source_hash,
        schema_namespace: CACHE_SCHEMA,
        font_engine_fingerprint: FONT_ENGINE_FINGERPRINT,
        raster_contract_id,
        contract_id,
        backend,
        supersample,
        base_size: TMP_POINT_SIZE,
        spread: TMP_SPREAD,
        atlas_width: atlas.page_width,
        atlas_height: atlas.page_height,
        glyphs,
        missing,
    })
}

fn identity(
    region: &str,
    font_source_hash: &str,
    glyph_index: u32,
    backend: RasterBackend,
    supersample: usize,
    raster_contract_id: &str,
) -> Result<GlyphRasterIdentity, String> {
    let canonical = json!([
        ["schema", CACHE_SCHEMA],
        ["region", region],
        ["font_sha256", font_source_hash.to_ascii_lowercase()],
        ["face_index", 0],
        ["variation_axes", json!([])],
        ["glyph_id", glyph_index],
        ["point_size_26d6", (TMP_POINT_SIZE as u32) * 64],
        ["dpi_x", 72],
        ["dpi_y", 72],
        ["load_flags", backend.load_flags()],
        ["render_mode", backend.render_mode()],
        ["spread_26d6", (TMP_SPREAD as u32) * 64],
        ["sdf_algorithm", backend.name()],
        ["supersample", supersample],
        ["threshold", THRESHOLD],
        ["downsample_version", DOWNSAMPLE_VERSION],
        ["font_engine", FONT_ENGINE_FINGERPRINT],
        ["raster_contract", raster_contract_id],
    ]);
    let bytes = serde_json::to_vec(&canonical).map_err(|error| error.to_string())?;
    Ok(GlyphRasterIdentity {
        opaque_key: format!("{:x}", Sha256::digest(bytes)),
        schema_namespace: CACHE_SCHEMA,
        font_engine_fingerprint: FONT_ENGINE_FINGERPRINT,
        raster_contract_id: raster_contract_id.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raster_contract_normalizes_supersample_before_identity() {
        assert_eq!(RasterBackend::Edt.normalized_supersample(99), 4);
        assert_eq!(RasterBackend::Analytic.normalized_supersample(99), 0);
    }

    #[test]
    fn identity_matches_the_existing_browser_cache_namespace() {
        let contract = "allium-r8-edt-ss2-spread6-threshold128-box-v1";
        let planned = identity("cn", &"a".repeat(64), 42, RasterBackend::Edt, 2, contract).unwrap();
        assert_eq!(
            planned.opaque_key,
            "fbd1658fe3bd464c0a54ee42284e7c15f64b68dd60d270c81e818de61bac9894"
        );
    }
}
