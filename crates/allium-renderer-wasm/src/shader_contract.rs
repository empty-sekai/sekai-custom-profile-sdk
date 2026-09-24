//! Pins the WebGL2 SDF, badge and uGUI glyph shaders to the formulas the
//! native executor evaluates, so the two backends cannot drift apart silently.

use sekai_profile_renderer_core::badge_material::{
    ALPHA_CUTOFF, DIELECTRIC_DIFFUSE, DISTRIBUTION_BIAS, LIGHTS, METALLIC, MIN_LIGHT_HALF_SQUARED,
    MIN_NORMAL_Z, MIN_ROUGHNESS, NORMALIZATION_BIAS, NORMALIZATION_SCALE, SMOOTHNESS,
    SPECULAR_FLOOR, SPECULAR_MAX, VIEW_DIRECTION,
};
use sekai_profile_renderer_core::sdf_material::{
    SHAPE_FACE_THRESHOLD, SHAPE_FACE_THRESHOLD_PER_OUTLINE, SHAPE_OUTER_FILL_RATIO,
    SHAPE_OUTLINE_THRESHOLD_PER_FILL, SHAPE_SDF_SHARPNESS,
};
use sekai_profile_renderer_core::ugui_text::TEXEL_CENTRE;

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

#[test]
fn badge_shader_uses_the_shared_badge_material() {
    let [view_x, view_y, view_z] = VIEW_DIRECTION;
    let mut statements = vec![
        format!("vec3 halfDir = normalize(light.xyz + vec3({view_x:?}, {view_y:?}, {view_z:?}));"),
        "float lightHalf = clamp(dot(light.xyz, halfDir), 0.0, 1.0);".to_string(),
        format!("float lightHalf2 = max(lightHalf * lightHalf, {MIN_LIGHT_HALF_SQUARED:?});"),
        "float normalHalf = clamp(dot(surface, halfDir), 0.0, 1.0);".to_string(),
        format!(
            "float d = normalHalf * normalHalf * (roughness2 - 1.0) + {DISTRIBUTION_BIAS:?};"
        ),
        "float term = roughness2 / (d * d * lightHalf2 * normalization);".to_string(),
        format!(
            "return clamp(term - {SPECULAR_FLOOR:?}, 0.0, {SPECULAR_MAX:?}) * light.w;"
        ),
        "vec4 albedo = texture(u_image, v_uv);".to_string(),
        "vec4 packedNormal = texture(u_normalMap, v_uv);".to_string(),
        "vec3 tangent = vec3(normalize(v_tangent), 0.0);".to_string(),
        "vec3 normal = vec3(0.0, 0.0, -1.0);".to_string(),
        "vec3 bitangent = cross(normal, tangent) * -1.0;".to_string(),
        "float nx = packedNormal.r * packedNormal.a * 2.0 - 1.0;".to_string(),
        "float ny = packedNormal.g * 2.0 - 1.0;".to_string(),
        format!("float nz = max(sqrt(1.0 - min(nx * nx + ny * ny, 1.0)), {MIN_NORMAL_Z:?});"),
        "vec3 surface = nx * tangent + ny * bitangent + nz * normal;".to_string(),
        format!(
            "float roughness = max((1.0 - {SMOOTHNESS:?}) * (1.0 - {SMOOTHNESS:?}), {MIN_ROUGHNESS:?});"
        ),
        "float roughness2 = roughness * roughness;".to_string(),
        format!(
            "float normalization = roughness * {NORMALIZATION_SCALE:?} + {NORMALIZATION_BIAS:?};"
        ),
        "float specular = 0.0;".to_string(),
        format!("float diffuse = {DIELECTRIC_DIFFUSE:?} * (1.0 - {METALLIC:?});"),
        format!(
            "float alpha = (albedo.a >= {ALPHA_CUTOFF:?} ? 1.0 : 0.0) * clamp(v_fill.a, 0.0, 1.0);"
        ),
        "vec3 color = clamp((albedo.rgb * diffuse + vec3(specular)) * v_fill.rgb * alpha, 0.0, 1.0);"
            .to_string(),
        "outColor = vec4(color, alpha);".to_string(),
        // The vertex stage turns the canvas axis (+y down) to the lighting
        // frame (+y up).
        "v_tangent = vec2(axis.x, -axis.y);".to_string(),
    ];
    statements.extend(LIGHTS.iter().map(|[x, y, z, intensity]| {
        format!(
            "specular += badgeSpecular(surface, vec4({x:?}, {y:?}, {z:?}, {intensity:?}), roughness2, normalization);"
        )
    }));
    for statement in &statements {
        assert_statement(SEMANTIC_EXECUTOR, statement);
    }
    // Exactly the shared lights, each added once.
    assert_eq!(
        SEMANTIC_EXECUTOR
            .lines()
            .filter(|line| line.trim().starts_with("specular += badgeSpecular("))
            .count(),
        LIGHTS.len()
    );
}

/// The uGUI glyph fragment shader, between its declaration and the next one.
fn ugui_glyph_shader() -> &'static str {
    let start = SEMANTIC_EXECUTOR
        .find("const UGUI_GLYPH_FRAGMENT_SHADER")
        .expect("uGUI glyph shader");
    let end = start
        + SEMANTIC_EXECUTOR[start..]
            .find("const COMPOSITE_VERTEX_SHADER")
            .expect("shader after the uGUI glyph shader");
    &SEMANTIC_EXECUTOR[start..end]
}

/// The statements of `ugui_text::cell_coverage` and of the native glyph
/// blend, in the shader's names: the cell texel at the fragment centre less
/// the texel centre, bilinear between the four nearest texels, empty outside
/// the bitmap, and the clamped vertex colour, premultiplied, times the
/// coverage.
#[test]
fn ugui_glyph_shader_samples_cells_like_the_shared_core() {
    let shader = ugui_glyph_shader();
    for statement in [
        format!("float x = v_uv.x - {TEXEL_CENTRE:?};"),
        format!("float y = v_uv.y - {TEXEL_CENTRE:?};"),
        "float left = floor(x);".to_string(),
        "float top = floor(y);".to_string(),
        "float fx = x - left;".to_string(),
        "float fy = y - top;".to_string(),
        "int column = int(left);".to_string(),
        "int row = int(top);".to_string(),
        "float upper = glyphTexel(column, row) + (glyphTexel(column + 1, row) - glyphTexel(column, row)) * fx;".to_string(),
        "float lower = glyphTexel(column, row + 1) + (glyphTexel(column + 1, row + 1) - glyphTexel(column, row + 1)) * fx;".to_string(),
        "float coverage = upper + (lower - upper) * fy;".to_string(),
        "if (coverage <= 0.0) discard;".to_string(),
        "int x = column - padding;".to_string(),
        "int y = row - padding;".to_string(),
        "if (x < 0 || y < 0 || x >= size.x || y >= size.y) return 0.0;".to_string(),
        "return texelFetch(u_glyphs, origin + ivec2(x, y), 0).r;".to_string(),
        "vec4 color = clamp(v_fill, 0.0, 1.0);".to_string(),
        "outColor = vec4(color.rgb * color.a, color.a) * coverage;".to_string(),
    ] {
        assert_statement(shader, &statement);
    }
    // Hardware filtering would quantise the interpolation weights.
    assert!(
        !shader.contains("texture("),
        "the glyph shader filters texels itself"
    );
    assert_statement(
        SEMANTIC_EXECUTOR,
        "programs.push(createProgram(gl, VERTEX_SHADER, UGUI_GLYPH_FRAGMENT_SHADER));",
    );
    assert_statement(
        SEMANTIC_EXECUTOR,
        "gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, page.width, page.height, 0, gl.RED, gl.UNSIGNED_BYTE, page.pixels);",
    );
}
