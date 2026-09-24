//! Pins the WebGL2 SDF, shape, image, badge and uGUI glyph shaders, and the
//! command clip they share, to the rules the native executor evaluates, so
//! the two backends cannot drift apart silently.

use sekai_profile_renderer_core::badge_material::{
    ALPHA_CUTOFF, DIELECTRIC_DIFFUSE, DISTRIBUTION_BIAS, LIGHTS, METALLIC, MIN_LIGHT_HALF_SQUARED,
    MIN_NORMAL_Z, MIN_ROUGHNESS, NORMALIZATION_BIAS, NORMALIZATION_SCALE, SMOOTHNESS,
    SPECULAR_FLOOR, SPECULAR_MAX, VIEW_DIRECTION,
};
use sekai_profile_renderer_core::pixel_sampling::{
    span_contains, CLIP_AXIS_TOLERANCE, GRADIENT_MIN_LENGTH_SQUARED, PIXEL_CENTRE, SHAPE_SAMPLES,
    SHAPE_SAMPLE_WEIGHT, TEXEL_CENTRE,
};
use sekai_profile_renderer_core::sdf_material::{
    SHAPE_FACE_THRESHOLD, SHAPE_FACE_THRESHOLD_PER_OUTLINE, SHAPE_OUTER_FILL_RATIO,
    SHAPE_OUTLINE_THRESHOLD_PER_FILL, SHAPE_SDF_SHARPNESS, TMP_MIN_SHADER_SCALE,
};

const SEMANTIC_EXECUTOR: &str = include_str!("gpu/webglSemanticCommandExecutor.ts");
const GLYPH_PIPELINE: &str = include_str!("gpu/webglSdfGlyphPipeline.ts");
const COMMAND_CLIP_SHADER: &str = include_str!("gpu/commandClipShader.ts");
const SEMANTIC_GEOMETRY: &str = include_str!("gpu/semanticCommandGeometry.ts");

/// The fragment stages of the semantic executor that map the pixel under the
/// fragment into the draw themselves.
const SEMANTIC_FRAGMENT_SHADERS: [&str; 4] = [
    "SHAPE_FRAGMENT_SHADER",
    "TEXTURE_FRAGMENT_SHADER",
    "BADGE_FRAGMENT_SHADER",
    "UGUI_GLYPH_FRAGMENT_SHADER",
];

/// The declarations of `source` that carry the storage qualifier `qualifier`
/// (`in` or `out`).
fn declarations<'a>(source: &'a str, qualifier: &str) -> Vec<&'a str> {
    source
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("//") && line.ends_with(';'))
        .filter(|line| line.split_whitespace().any(|token| token == qualifier))
        .collect()
}

fn assert_statement(source: &str, statement: &str) {
    assert!(
        source.lines().any(|line| line.trim() == statement),
        "shader statement missing: {statement}"
    );
}

/// The template literal of the GLSL constant `name` in `module`.
fn template(module: &'static str, name: &str) -> &'static str {
    let start = module
        .find(&format!("const {name} = `"))
        .unwrap_or_else(|| panic!("shader {name}"));
    let end = start
        + module[start..]
            .find("`;")
            .unwrap_or_else(|| panic!("end of shader {name}"));
    &module[start..end]
}

/// The executor's GLSL constant `name`.
fn shader(name: &str) -> &'static str {
    template(SEMANTIC_EXECUTOR, name)
}

/// `QUAD_OUTSET` of a vertex stage.
fn quad_outset(vertex: &str) -> f32 {
    vertex
        .lines()
        .find_map(|line| line.trim().strip_prefix("const float QUAD_OUTSET = "))
        .and_then(|value| value.strip_suffix(';'))
        .expect("QUAD_OUTSET")
        .parse()
        .expect("QUAD_OUTSET value")
}

/// The snapping of a vertex to the coarsest sub-pixel grid OpenGL ES allows
/// (4 bits), which a grown quad must absorb.
const VERTEX_SNAP: f32 = 1.0 / 16.0;

#[test]
fn shape_mask_shader_uses_the_shared_shape_material() {
    let texture = shader("TEXTURE_FRAGMENT_SHADER");
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
        assert_statement(texture, &statement);
    }
}

/// `shade_text` of the native tile executor: the face and underlay ramps,
/// the underlay below the face, times the vertex alpha.
#[test]
fn glyph_shader_composites_the_underlay_below_the_face() {
    let fragment = template(GLYPH_PIPELINE, "FRAGMENT_SHADER");
    for statement in [
        "float faceT = clamp(sdf * v_sdfParams.x - v_sdfParams.y, 0.0, 1.0);",
        "float underlayT = clamp(sdf * v_sdfParams.z - v_sdfParams.w, 0.0, 1.0);",
        "vec4 face = vec4(v_color.rgb * v_color.a, v_color.a);",
        "vec4 outline = vec4(v_outline.rgb * v_outline.a, v_outline.a);",
        "float outlineWeight = underlayT * (1.0 - face.a * faceT);",
        "outColor = (outline * outlineWeight + face * faceT) * v_vertexAlpha;",
    ] {
        assert_statement(fragment, statement);
    }
    let vertex = template(GLYPH_PIPELINE, "VERTEX_SHADER");
    for statement in [
        format!("const float MIN_SHADER_SCALE = {TMP_MIN_SHADER_SCALE:?};"),
        "v_sdfParams = vec4(max(a_sdfParams.x, MIN_SHADER_SCALE), a_sdfParams.y, max(a_sdfParams.z, MIN_SHADER_SCALE), a_sdfParams.w);".to_string(),
        "v_vertexAlpha = clamp(a_instanceMeta.x, 0.0, 1.0);".to_string(),
    ] {
        assert_statement(vertex, &statement);
    }
}

/// The SDF glyph stage samples like the native tile executor: the pixel
/// centre, found from the fragment position, is drawn when it lies in the
/// clip and in the glyph quad by the executor's scanline rule, and maps into
/// the atlas through the glyph's canvas-to-texel matrix in the executor's
/// order of operations; the distance field is filtered bilinearly between
/// the four texels whose centres surround the sample, clamped to the glyph's
/// rectangle.
#[test]
fn sdf_glyph_shader_samples_like_the_native_tile_executor() {
    let fragment = template(GLYPH_PIPELINE, "FRAGMENT_SHADER");
    for statement in [
        "vec2 window = floor(gl_FragCoord.xy);".to_string(),
        "return vec2(window.x, ${CARD_H - 1}.0 - window.y);".to_string(),
        format!("vec2 centre = canvasPixel() + vec2({PIXEL_CENTRE:?});"),
        "if (!insideClip(centre) || !quadCovers(centre)) discard;".to_string(),
        // `plan_command` and the scalar sampler.
        "v_toTexel.x * centre.x + (v_toTexel.z * centre.y + v_toTexelOffset.x),".to_string(),
        "v_toTexel.y * centre.x + (v_toTexel.w * centre.y + v_toTexelOffset.y)".to_string(),
        // `sample_bilinear` and `shade_sample`.
        "vec2 base = floor(texel);".to_string(),
        "vec2 weight = texel - base;".to_string(),
        "ivec2 first = ivec2(v_rect.xy);".to_string(),
        "ivec2 last = first + ivec2(v_rect.zw) - 1;".to_string(),
        "ivec2 low = clamp(ivec2(base), first, last);".to_string(),
        "ivec2 high = clamp(ivec2(base) + 1, first, last);".to_string(),
        "float p00 = atlasTexel(low);".to_string(),
        "float p10 = atlasTexel(ivec2(high.x, low.y));".to_string(),
        "float p01 = atlasTexel(ivec2(low.x, high.y));".to_string(),
        "float p11 = atlasTexel(high);".to_string(),
        "float top = (p10 - p00) * weight.x + p00;".to_string(),
        "float bottom = (p11 - p01) * weight.x + p01;".to_string(),
        "float sdf = (bottom - top) * weight.y + top;".to_string(),
        "return texelFetch(u_atlas, ivec3(texel, v_atlasPage), 0).r;".to_string(),
        // `scan_command`: each side crossing the row, half-open in y, bounds a
        // span that holds [min, max).
        "if (point.y < bottom || point.y >= top || top == bottom) continue;".to_string(),
        "float t = (point.y - start.y) / (end.y - start.y);".to_string(),
        "float x = (end.x - start.x) * t + start.x;".to_string(),
        "return crossings >= 2 && point.x >= low && point.x < high;".to_string(),
    ] {
        assert_statement(fragment, &statement);
    }
    assert!(fragment.contains("${COMMAND_CLIP_GLSL}"));
    assert!(
        !fragment.contains("texture("),
        "the glyph stage filters in hardware"
    );
    let inputs = declarations(fragment, "in");
    assert!(inputs.len() >= 12, "{inputs:?}");
    assert!(
        inputs.iter().all(|line| line.starts_with("flat in ")),
        "every fragment input is per glyph: {inputs:?}"
    );
    let vertex = template(GLYPH_PIPELINE, "VERTEX_SHADER");
    let outputs = declarations(vertex, "out");
    assert!(
        !outputs.is_empty() && outputs.iter().all(|line| line.starts_with("flat out ")),
        "every vertex output is per glyph: {outputs:?}"
    );
    // The quad spans the glyph's rectangle edge to edge, texel centres at
    // whole texel coordinates, on the GPU and in the CPU's matrix.
    for statement in [
        format!("const float TEXEL_CENTRE = {TEXEL_CENTRE:?};"),
        "vec2 texel = a_rect.xy - TEXEL_CENTRE + unit * a_rect.zw + (unit * 2.0 - 1.0) * outset;"
            .to_string(),
    ] {
        assert_statement(vertex, &statement);
    }
    for statement in [
        format!("const TEXEL_CENTRE = {TEXEL_CENTRE:?};"),
        "fma(width, aC, f32(x - TEXEL_CENTRE)),".to_string(),
        "fma(height, bC, f32(y - TEXEL_CENTRE)),".to_string(),
    ] {
        assert_statement(GLYPH_PIPELINE, &statement);
    }
}

/// A port of `clipSideKeeps` and `insideClip` of the shared clip GLSL, whose
/// statements `command_clips_keep_the_shared_half_open_bounds` pins.
fn glsl_inside_clip(corners: [[f32; 2]; 4], point: [f32; 2]) -> bool {
    let cross2 = |a: [f32; 2], b: [f32; 2]| a[0] * b[1] - a[1] * b[0];
    let sub = |a: [f32; 2], b: [f32; 2]| [a[0] - b[0], a[1] - b[1]];
    let winding = if cross2(sub(corners[1], corners[0]), sub(corners[3], corners[0])) < 0.0 {
        -1.0
    } else {
        1.0
    };
    (0..4).all(|index| {
        let start = corners[index];
        let end = corners[(index + 1) % 4];
        let side = [(end[0] - start[0]) * winding, (end[1] - start[1]) * winding];
        let turn = cross2(side, sub(point, start));
        turn > 0.0 || (turn == 0.0 && (side[1] < 0.0 || (side[1] == 0.0 && side[0] > 0.0)))
    })
}

/// The fragment stages keep `pixel_sampling::span_contains` of the clip's
/// bounds on both axes: a clip reduced to its axis-aligned bounds keeps its
/// top and left edges and drops its bottom and right ones, whichever way its
/// corners run.
#[test]
fn command_clips_keep_the_shared_half_open_bounds() {
    let clip = template(COMMAND_CLIP_SHADER, "COMMAND_CLIP_GLSL");
    for statement in [
        "vec2 side = (end - start) * winding;",
        "float turn = cross2(side, point - start);",
        "return turn > 0.0 || (turn == 0.0 && (side.y < 0.0 || (side.y == 0.0 && side.x > 0.0)));",
        "float winding = cross2(p[1] - p[0], p[3] - p[0]) < 0.0 ? -1.0 : 1.0;",
        "return clipSideKeeps(p[0], p[1], point, winding)",
        "&& clipSideKeeps(p[1], p[2], point, winding)",
        "&& clipSideKeeps(p[2], p[3], point, winding)",
        "&& clipSideKeeps(p[3], p[0], point, winding);",
    ] {
        assert_statement(clip, statement);
    }
    let [min_x, min_y, max_x, max_y] = [10.0f32, 20.5, 14.5, 23.0];
    let clockwise = [
        [min_x, min_y],
        [max_x, min_y],
        [max_x, max_y],
        [min_x, max_y],
    ];
    let mut anticlockwise = clockwise;
    anticlockwise.reverse();
    for corners in [clockwise, anticlockwise] {
        for y in [19.5f32, 20.0, 20.5, 21.0, 22.5, 23.0, 23.5] {
            for x in [9.5f32, 10.0, 10.5, 14.0, 14.5, 15.0] {
                assert_eq!(
                    glsl_inside_clip(corners, [x, y]),
                    span_contains(min_x, max_x, x) && span_contains(min_y, max_y, y),
                    "{corners:?} ({x}, {y})"
                );
            }
        }
    }
    // Both fragment families use it, on the pixel centre.
    assert!(shader("FRAGMENT_COMMON").contains("${COMMAND_CLIP_GLSL}"));
    assert!(template(GLYPH_PIPELINE, "FRAGMENT_SHADER").contains("${COMMAND_CLIP_GLSL}"));
    // The geometry reduces a clip to its bounds with the shared tolerance.
    assert_statement(
        SEMANTIC_GEOMETRY,
        &format!("export const CLIP_AXIS_TOLERANCE = Math.fround({CLIP_AXIS_TOLERANCE:?});"),
    );
}

/// Every semantic fragment stage takes its sampling positions from the
/// fragment's window position through the draw's inverse matrix, never from
/// interpolated varyings, and reads texels with `texelFetch`.
#[test]
fn fragment_stages_sample_at_the_fragment_position() {
    let common = shader("FRAGMENT_COMMON");
    assert!(common.contains("${COMMAND_CLIP_GLSL}"));
    for statement in [
        "vec2 window = floor(gl_FragCoord.xy);",
        // The canvas's +y runs down, the window's up.
        "return vec2(window.x, u_canvas.y - 1.0 - window.y);",
        // `transform_point` of the native compositor.
        "v_inverse.x * point.x + (v_inverse.z * point.y + v_inverseOffset.x),",
        "v_inverse.y * point.x + (v_inverse.w * point.y + v_inverseOffset.y)",
    ] {
        assert_statement(common, statement);
    }
    let inputs = declarations(common, "in");
    assert!(inputs.len() >= 13, "{inputs:?}");
    assert!(
        inputs.iter().all(|line| line.starts_with("flat in ")),
        "every fragment input is per draw: {inputs:?}"
    );
    let outputs = declarations(shader("VERTEX_SHADER"), "out");
    assert!(!outputs.is_empty());
    assert!(
        outputs.iter().all(|line| line.starts_with("flat out ")),
        "every vertex output is per draw: {outputs:?}"
    );
    for name in SEMANTIC_FRAGMENT_SHADERS {
        let source = shader(name);
        assert!(source.contains("${FRAGMENT_COMMON}"), "{name}");
        assert!(
            declarations(source, "in").is_empty(),
            "{name} declares its own inputs"
        );
        assert!(!source.contains("texture("), "{name} filters in hardware");
        assert!(
            !source.contains("dFdx") && !source.contains("fwidth"),
            "{name}"
        );
    }
    assert_statement(
        shader("COMPOSITE_FRAGMENT_SHADER"),
        "outColor = texelFetch(u_image, ivec2(gl_FragCoord.xy), 0);",
    );
}

/// The vertex stages grow each quad so that every pixel with a sample in the
/// draw is rasterised, whatever the rasteriser's vertex snapping.
#[test]
fn quads_reach_every_sampled_pixel() {
    let vertex = shader("VERTEX_SHADER");
    let outset = quad_outset(vertex);
    // The farthest sample from its pixel's centre, plus the snapping.
    let reach = SHAPE_SAMPLES
        .iter()
        .map(|[x, y]| (x - PIXEL_CENTRE).hypot(y - PIXEL_CENTRE))
        .fold(0.0, f32::max);
    assert!(outset >= reach + VERTEX_SNAP, "{outset} < {reach} + 1/16");
    for statement in [
        "vec2 outset = QUAD_OUTSET * vec2(length(inverse.xz), length(inverse.yw));",
        "vec2 local = a_bounds.xy + a_corner * a_bounds.zw + (a_corner * 2.0 - 1.0) * outset;",
    ] {
        assert_statement(vertex, statement);
    }
    // A glyph is sampled at the pixel centre alone.
    let glyph = template(GLYPH_PIPELINE, "VERTEX_SHADER");
    assert!(quad_outset(glyph) >= VERTEX_SNAP);
    assert_statement(
        glyph,
        "vec2 outset = QUAD_OUTSET * vec2(length(toTexel.xz), length(toTexel.yw));",
    );
}

/// The statements of `pixel_sampling::shape_contains`, `shape_fill` and the
/// native shape raster, in the shader's names.
#[test]
fn shape_shader_counts_the_shared_samples() {
    let shape = shader("SHAPE_FRAGMENT_SHADER");
    let mut statements = vec![
        format!("highp vec2 samples[{}];", SHAPE_SAMPLES.len()),
        format!(
            "for (int index = 0; index < {}; index += 1) {{",
            SHAPE_SAMPLES.len()
        ),
        "vec2 pixel = canvasPixel();".to_string(),
        format!("if (!insideClip(pixel + vec2({PIXEL_CENTRE:?}))) discard;"),
        "vec2 point = toLocal(pixel + samples[index]);".to_string(),
        "if (!shapeContains(point, 0.0)) continue;".to_string(),
        "covered = true;".to_string(),
        "bool useStroke = v_params.w > 0.0 && !shapeContains(point, v_params.w);".to_string(),
        "vec4 color = useStroke ? v_stroke : shapeFill(point);".to_string(),
        "float alpha = clamp(color.a, 0.0, 1.0);".to_string(),
        format!(
            "accumulated += vec4(clamp(color.rgb, 0.0, 1.0) * alpha, alpha) * {SHAPE_SAMPLE_WEIGHT:?};"
        ),
        "if (!covered) discard;".to_string(),
        "outColor = accumulated;".to_string(),
        // shape_contains
        "float left = v_bounds.x + inset;".to_string(),
        "float top = v_bounds.y + inset;".to_string(),
        "float right = v_bounds.x + v_bounds.z - inset;".to_string(),
        "float bottom = v_bounds.y + v_bounds.w - inset;".to_string(),
        "if (left >= right || top >= bottom || point.x < left || point.x >= right || point.y < top || point.y >= bottom) return false;".to_string(),
        "float rx = (right - left) * 0.5;".to_string(),
        "float ry = (bottom - top) * 0.5;".to_string(),
        "float nx = (point.x - (left + right) * 0.5) / rx;".to_string(),
        "float ny = (point.y - (top + bottom) * 0.5) / ry;".to_string(),
        "return nx * nx + ny * ny <= 1.0;".to_string(),
        "float rx = min(max(v_params.y - inset, 0.0), (right - left) * 0.5);".to_string(),
        "float ry = min(max(v_params.z - inset, 0.0), (bottom - top) * 0.5);".to_string(),
        "if (rx == 0.0 || ry == 0.0) return true;".to_string(),
        "float cx = clamp(point.x, left + rx, right - rx);".to_string(),
        "float cy = clamp(point.y, top + ry, bottom - ry);".to_string(),
        "float nx = (point.x - cx) / rx;".to_string(),
        "float ny = (point.y - cy) / ry;".to_string(),
        // shape_fill
        "float u = (point.x - v_bounds.x) / v_bounds.z;".to_string(),
        "float v = (point.y - v_bounds.y) / v_bounds.w;".to_string(),
        "float dx = v_gradient.z - v_gradient.x;".to_string(),
        "float dy = v_gradient.w - v_gradient.y;".to_string(),
        "float denominator = dx * dx + dy * dy;".to_string(),
        format!(
            "float t = denominator <= {GRADIENT_MIN_LENGTH_SQUARED:?} ? 0.0 : clamp(((u - v_gradient.x) * dx + (v - v_gradient.y) * dy) / denominator, 0.0, 1.0);"
        ),
        "return (v_gradientEndColor - v_fill) * t + v_fill;".to_string(),
    ];
    statements.extend(
        SHAPE_SAMPLES
            .iter()
            .enumerate()
            .map(|(index, [x, y])| format!("samples[{index}] = vec2({x:?}, {y:?});")),
    );
    for statement in &statements {
        assert_statement(shape, statement);
    }
    // Exactly the shared samples.
    assert_eq!(
        shape
            .lines()
            .filter(|line| line.trim().starts_with("samples["))
            .count(),
        SHAPE_SAMPLES.len()
    );
}

/// Images, masks and badges: one sample at the pixel centre, drawn in the
/// half-open bounds and the image clip (`pixel_sampling::ellipse_clip_contains`
/// and `rounded_rect_clip_contains`), reading the texel `nearest_texel`
/// selects; an alpha mask is read the same way at the position in the bounds.
#[test]
fn image_shaders_read_the_texel_under_the_pixel_centre() {
    let sampling = shader("IMAGE_SAMPLING");
    for statement in [
        "if (v_params.x < 0.5) return true;",
        "float halfWidth = v_bounds.z * 0.5;",
        "float halfHeight = v_bounds.w * 0.5;",
        "if (halfWidth <= 0.0 || halfHeight <= 0.0) return false;",
        "float ellipseX = (point.x - (v_bounds.x + halfWidth)) / halfWidth;",
        "float ellipseY = (point.y - (v_bounds.y + halfHeight)) / halfHeight;",
        "return ellipseX * ellipseX + ellipseY * ellipseY <= 1.0;",
        "float radiusX = min(abs(v_params.y), halfWidth);",
        "float radiusY = min(abs(v_params.z), halfHeight);",
        "if (radiusX == 0.0 || radiusY == 0.0) return true;",
        "float distanceX = abs(point.x - (v_bounds.x + halfWidth)) - (halfWidth - radiusX);",
        "float distanceY = abs(point.y - (v_bounds.y + halfHeight)) - (halfHeight - radiusY);",
        "if (distanceX <= 0.0 || distanceY <= 0.0) return true;",
        "float cornerX = distanceX / radiusX;",
        "float cornerY = distanceY / radiusY;",
        "return cornerX * cornerX + cornerY * cornerY <= 1.0;",
        // nearest_texel
        "vec2 texel = floor(coordinate * vec2(size));",
        "return ivec2(clamp(texel, vec2(0.0), vec2(size - 1)));",
    ] {
        assert_statement(sampling, statement);
    }
    for name in ["TEXTURE_FRAGMENT_SHADER", "BADGE_FRAGMENT_SHADER"] {
        let source = shader(name);
        assert!(source.contains("${IMAGE_SAMPLING}"), "{name}");
        for statement in [
            format!("vec2 centre = canvasPixel() + vec2({PIXEL_CENTRE:?});"),
            "if (!insideClip(centre)) discard;".to_string(),
            "vec2 local = toLocal(centre);".to_string(),
            "float u = (local.x - v_bounds.x) / v_bounds.z;".to_string(),
            "float v = (local.y - v_bounds.y) / v_bounds.w;".to_string(),
            "if (u < 0.0 || u >= 1.0 || v < 0.0 || v >= 1.0) discard;".to_string(),
            "if (!imageClipContains(local)) discard;".to_string(),
            "vec2 imageUv = v_uvRect.xy + vec2(u, v) * v_uvRect.zw;".to_string(),
            "if (maskCoverage == 0.0) discard;".to_string(),
        ] {
            assert_statement(source, &statement);
        }
    }
    let texture = shader("TEXTURE_FRAGMENT_SHADER");
    for statement in [
        "vec4 sampleColor = texelFetch(u_image, nearestTexel(imageUv, textureSize(u_image, 0)), 0);",
        "maskCoverage = texelFetch(u_alphaMask, nearestTexel(vec2(u, v), textureSize(u_alphaMask, 0)), 0).a;",
        "vec4 tint = clamp(v_fill, 0.0, 1.0);",
        "outColor = color * maskCoverage;",
    ] {
        assert_statement(texture, statement);
    }
}

#[test]
fn badge_shader_uses_the_shared_badge_material() {
    let badge = shader("BADGE_FRAGMENT_SHADER");
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
        // The albedo is the texel under the pixel centre; the normal map is
        // filtered bilinearly at the same image position, as
        // `pixel_sampling::bilinear_taps` weighs it.
        "vec4 albedo = texelFetch(u_image, nearestTexel(imageUv, textureSize(u_image, 0)), 0);"
            .to_string(),
        "vec4 packedNormal = normalTexel(imageUv);".to_string(),
        "ivec2 size = textureSize(u_normalMap, 0);".to_string(),
        "vec2 last = vec2(size - 1);".to_string(),
        format!(
            "vec2 position = clamp(coordinate * vec2(size) - {TEXEL_CENTRE:?}, vec2(0.0), last);"
        ),
        "vec2 low = floor(position);".to_string(),
        "vec2 high = min(low + 1.0, last);".to_string(),
        "vec2 weight = position - low;".to_string(),
        "vec4 top = texelFetch(u_normalMap, ivec2(low.x, low.y), 0) * (1.0 - weight.x) + texelFetch(u_normalMap, ivec2(high.x, low.y), 0) * weight.x;".to_string(),
        "vec4 bottom = texelFetch(u_normalMap, ivec2(low.x, high.y), 0) * (1.0 - weight.x) + texelFetch(u_normalMap, ivec2(high.x, high.y), 0) * weight.x;".to_string(),
        "return top * (1.0 - weight.y) + bottom * weight.y;".to_string(),
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
        "outColor *= maskCoverage;".to_string(),
    ];
    statements.extend(LIGHTS.iter().map(|[x, y, z, intensity]| {
        format!(
            "specular += badgeSpecular(surface, vec4({x:?}, {y:?}, {z:?}, {intensity:?}), roughness2, normalization);"
        )
    }));
    for statement in &statements {
        assert_statement(badge, statement);
    }
    // The vertex stage turns the canvas direction of the local +x axis (+y
    // down) to the lighting frame (+y up).
    let vertex = shader("VERTEX_SHADER");
    assert_statement(vertex, "vec2 axis = forward.xy;");
    assert_statement(vertex, "v_tangent = vec2(axis.x, -axis.y);");
    // Exactly the shared lights, each added once.
    assert_eq!(
        badge
            .lines()
            .filter(|line| line.trim().starts_with("specular += badgeSpecular("))
            .count(),
        LIGHTS.len()
    );
}

/// The statements of `ugui_text::cell_coverage` and of the native glyph
/// raster, in the shader's names: the cell position of the pixel centre, drawn
/// when it lies in the cell, less the texel centre, bilinear between the four
/// nearest texels, empty outside the bitmap, and the clamped vertex colour,
/// premultiplied, times the coverage.
#[test]
fn ugui_glyph_shader_samples_cells_like_the_shared_core() {
    let glyph = shader("UGUI_GLYPH_FRAGMENT_SHADER");
    for statement in [
        format!("vec2 centre = canvasPixel() + vec2({PIXEL_CENTRE:?});"),
        "vec2 cell = toLocal(centre);".to_string(),
        "if (cell.x < 0.0 || cell.x >= v_bounds.z || cell.y < 0.0 || cell.y >= v_bounds.w) discard;"
            .to_string(),
        format!("float x = cell.x - {TEXEL_CENTRE:?};"),
        format!("float y = cell.y - {TEXEL_CENTRE:?};"),
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
        assert_statement(glyph, &statement);
    }
    assert_statement(
        SEMANTIC_EXECUTOR,
        "programs.push(createProgram(gl, VERTEX_SHADER, UGUI_GLYPH_FRAGMENT_SHADER));",
    );
    assert_statement(
        SEMANTIC_EXECUTOR,
        "gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, page.width, page.height, 0, gl.RED, gl.UNSIGNED_BYTE, page.pixels);",
    );
}
