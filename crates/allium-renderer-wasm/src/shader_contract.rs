//! Pins the WebGL2 SDF shaders to the material formulas the native executor
//! evaluates, so the two backends cannot drift apart silently.

use sekai_profile_renderer_core::sdf_material::{
    SHAPE_FACE_THRESHOLD, SHAPE_FACE_THRESHOLD_PER_OUTLINE, SHAPE_OUTER_FILL_RATIO,
    SHAPE_OUTLINE_THRESHOLD_PER_FILL, SHAPE_SDF_SHARPNESS,
};

const SEMANTIC_EXECUTOR: &str = include_str!("gpu/webglSemanticCommandExecutor.ts");
const GLYPH_PIPELINE: &str = include_str!("gpu/webglSdfGlyphPipeline.ts");

fn assert_statement(source: &str, statement: &str) {
    assert!(
        source.lines().any(|line| line.trim() == statement),
        "shader statement missing: {statement}"
    );
}

#[test]
fn shape_mask_shader_uses_the_shared_shape_material() {
    for statement in [
        "float outlineSize = clamp(v_params.w, 0.0, 1.0);".to_string(),
        format!(
            "float faceThreshold = {SHAPE_FACE_THRESHOLD:?} + outlineSize * {SHAPE_FACE_THRESHOLD_PER_OUTLINE:?};"
        ),
        format!(
            "float outlineThreshold = min(1.0 - outlineSize * {SHAPE_OUTER_FILL_RATIO:?} * {SHAPE_OUTLINE_THRESHOLD_PER_FILL:?}, {SHAPE_FACE_THRESHOLD:?});"
        ),
        format!("float sharp = {SHAPE_SDF_SHARPNESS:?} / 255.0;"),
        "float faceAlpha = faceCoverage * roundEven(clamp(v_fill.a, 0.0, 1.0) * 255.0) / 255.0;"
            .to_string(),
        "float outlineAlpha = outlineCoverage * clamp(v_stroke.a, 0.0, 1.0);".to_string(),
    ] {
        assert_statement(SEMANTIC_EXECUTOR, &statement);
    }
}

#[test]
fn glyph_shader_composites_the_underlay_below_the_face() {
    for statement in [
        "float faceT = clamp(sdf * v_faceScale - v_faceBias, 0.0, 1.0);",
        "float underlayT = clamp(sdf * v_underlayScale - v_underlayBias, 0.0, 1.0);",
        "float oneMinusFace = 1.0 - face.a * faceT;",
        "outColor = (face * faceT + outline * underlayT * oneMinusFace) * v_vertexAlpha;",
    ] {
        assert_statement(GLYPH_PIPELINE, statement);
    }
}
