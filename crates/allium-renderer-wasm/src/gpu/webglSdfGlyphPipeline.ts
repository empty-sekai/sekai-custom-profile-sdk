import type { GlyphInstance } from "../types/glyph.js";
import { COMMAND_CLIP_GLSL } from "./commandClipShader.js";
import { NO_CLIP, type ClipQuad } from "./semanticCommandGeometry.js";

const CARD_W = 1830;
const CARD_H = 812;

/** One float attribute of a glyph instance: its shader location, its
 * component count and its offset in floats. */
export type SdfGlyphInstanceAttribute = { location: number; size: number; offset: number };

/** Float attributes of a glyph instance. The fragment stage maps each device
 * pixel into the glyph's atlas texels itself and decides there what it
 * shows, as the native tile executor does.
 *
 * - `quad01`, `quad23`: the glyph quad's top-left, top-right, bottom-right
 *   and bottom-left corners on the canvas, before the dynamic offsets and the
 *   preview transform the vertex stage applies.
 * - `toTexel`, `toTexelOffset`: the canvas-to-texel matrix of the glyph, `[a,
 *   b, c, d]` and `[e, f]` (`x' = a x + c y + e`; see `glyphTexelMap`).
 * - `rect`: the glyph's rectangle on its atlas page, in texels (x, y, width,
 *   height).
 * - `color`, `outline`: the face and underlay colours, straight RGBA.
 * - `sdfParams`: the face scale and bias and the underlay scale and bias.
 * - `instanceMeta`: the vertex alpha, the layer's state slot, the atlas page
 *   and the command's state slot.
 * - `clip01`, `clip23`: the command's clip quad (see `commandClipQuad`). */
export const SDF_GLYPH_INSTANCE_ATTRIBUTES = {
  quad01: { location: 0, size: 4, offset: 0 },
  quad23: { location: 1, size: 4, offset: 4 },
  toTexel: { location: 2, size: 4, offset: 8 },
  toTexelOffset: { location: 3, size: 2, offset: 12 },
  rect: { location: 4, size: 4, offset: 14 },
  color: { location: 5, size: 4, offset: 18 },
  outline: { location: 6, size: 4, offset: 22 },
  sdfParams: { location: 7, size: 4, offset: 26 },
  instanceMeta: { location: 8, size: 4, offset: 30 },
  clip01: { location: 9, size: 4, offset: 34 },
  clip23: { location: 10, size: 4, offset: 38 },
} as const satisfies Record<string, SdfGlyphInstanceAttribute>;
export const SDF_GLYPH_FLOATS_PER_INSTANCE = 42;

type GlyphGpuBatch = {
  vao: WebGLVertexArrayObject;
  buffer: WebGLBuffer;
  instances: number;
  bytes: number;
};

type GlyphUniforms = {
  atlas: WebGLUniformLocation | null;
  layerState: WebGLUniformLocation | null;
  layerStateWidth: WebGLUniformLocation | null;
  renderMask: WebGLUniformLocation | null;
  commandMask: WebGLUniformLocation | null;
  commandState: WebGLUniformLocation | null;
  commandWidth: WebGLUniformLocation | null;
  previewTransform: WebGLUniformLocation | null;
};

export class WebglSdfGlyphPipeline {
  private readonly gl: WebGL2RenderingContext;
  private readonly program: WebGLProgram;
  private readonly uniforms: GlyphUniforms;
  private readonly batches = new Map<string, GlyphGpuBatch>();
  private atlasTexture: WebGLTexture;
  private ownsAtlas = true;
  private geometryBuilds = 0;

  constructor(gl: WebGL2RenderingContext) {
    this.gl = gl;
    this.program = createProgram(gl, VERTEX_SHADER, FRAGMENT_SHADER);
    const uniform = (name: string) => gl.getUniformLocation(this.program, name);
    this.uniforms = {
      atlas: uniform("u_atlas"),
      layerState: uniform("u_layerState"),
      layerStateWidth: uniform("u_layerStateWidth"),
      renderMask: uniform("u_renderMask"),
      commandMask: uniform("u_commandMask"),
      commandState: uniform("u_commandState"),
      commandWidth: uniform("u_commandWidth"),
      previewTransform: uniform("u_previewTransform"),
    };
    const atlas = gl.createTexture();
    if (!atlas) throw new Error("glyph pipeline atlas allocation failed");
    this.atlasTexture = atlas;
    gl.bindTexture(gl.TEXTURE_2D_ARRAY, atlas);
    gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texImage3D(gl.TEXTURE_2D_ARRAY, 0, gl.R8, 1, 1, 1, 0, gl.RED, gl.UNSIGNED_BYTE, new Uint8Array([255]));
  }

  setAtlas(texture: WebGLTexture): void {
    if (this.ownsAtlas) this.gl.deleteTexture(this.atlasTexture);
    this.atlasTexture = texture;
    this.ownsAtlas = false;
  }

  upload(key: string, vertices: Float32Array): void {
    this.deleteBatch(key);
    if (vertices.length % SDF_GLYPH_FLOATS_PER_INSTANCE !== 0) throw new Error(`invalid glyph instance buffer ${key}`);
    const gl = this.gl;
    const vao = gl.createVertexArray();
    const buffer = gl.createBuffer();
    if (!vao || !buffer) throw new Error("glyph pipeline batch allocation failed");
    gl.bindVertexArray(vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(gl.ARRAY_BUFFER, vertices, gl.STATIC_DRAW);
    const stride = SDF_GLYPH_FLOATS_PER_INSTANCE * 4;
    for (const attribute of Object.values(SDF_GLYPH_INSTANCE_ATTRIBUTES)) {
      instanceAttribute(gl, attribute.location, attribute.size, stride, attribute.offset * 4);
    }
    gl.bindVertexArray(null);
    this.batches.set(key, { vao, buffer, instances: vertices.length / SDF_GLYPH_FLOATS_PER_INSTANCE, bytes: vertices.byteLength });
    this.geometryBuilds += 1;
  }

  draw(
    key: string,
    stateTexture: WebGLTexture,
    maskTexture: WebGLTexture,
    stateWidth: number,
    commandMaskTexture: WebGLTexture = maskTexture,
    commandStateTexture: WebGLTexture = stateTexture,
    commandWidth: number = stateWidth,
    previewTransformTexture: WebGLTexture = stateTexture,
  ): { drawCalls: number; instances: number; bytes: number } {
    const batch = this.batches.get(key);
    if (!batch || batch.instances === 0) return { drawCalls: 0, instances: 0, bytes: batch?.bytes ?? 0 };
    const gl = this.gl;
    const uniforms = this.uniforms;
    gl.useProgram(this.program);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D_ARRAY, this.atlasTexture);
    gl.uniform1i(uniforms.atlas, 0);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, stateTexture);
    gl.uniform1i(uniforms.layerState, 1);
    gl.uniform1f(uniforms.layerStateWidth, stateWidth);
    gl.activeTexture(gl.TEXTURE2);
    gl.bindTexture(gl.TEXTURE_2D, maskTexture);
    gl.uniform1i(uniforms.renderMask, 2);
    gl.activeTexture(gl.TEXTURE3);
    gl.bindTexture(gl.TEXTURE_2D, commandMaskTexture);
    gl.uniform1i(uniforms.commandMask, 3);
    gl.activeTexture(gl.TEXTURE4);
    gl.bindTexture(gl.TEXTURE_2D, commandStateTexture);
    gl.uniform1i(uniforms.commandState, 4);
    gl.uniform1f(uniforms.commandWidth, commandWidth);
    gl.activeTexture(gl.TEXTURE5);
    gl.bindTexture(gl.TEXTURE_2D, previewTransformTexture);
    gl.uniform1i(uniforms.previewTransform, 5);
    gl.bindVertexArray(batch.vao);
    gl.drawArraysInstanced(gl.TRIANGLES, 0, 6, batch.instances);
    gl.bindVertexArray(null);
    return { drawCalls: 1, instances: batch.instances, bytes: batch.bytes };
  }

  stats(): { geometryBuilds: number; batches: number; bytes: number } {
    return {
      geometryBuilds: this.geometryBuilds,
      batches: this.batches.size,
      bytes: [...this.batches.values()].reduce((sum, batch) => sum + batch.bytes, 0),
    };
  }

  clearBatches(): void {
    for (const key of [...this.batches.keys()]) this.deleteBatch(key);
  }

  destroy(): void {
    this.clearBatches();
    if (this.ownsAtlas) this.gl.deleteTexture(this.atlasTexture);
    this.gl.deleteProgram(this.program);
  }

  private deleteBatch(key: string): void {
    const batch = this.batches.get(key);
    if (!batch) return;
    this.gl.deleteBuffer(batch.buffer);
    this.gl.deleteVertexArray(batch.vao);
    this.batches.delete(key);
  }
}

/** Size of an atlas page in texels. */
export type SdfAtlasPageSize = { width: number; height: number };

/** Instance attributes of the drawable glyphs, laid out as
 * `SDF_GLYPH_INSTANCE_ATTRIBUTES`. A glyph whose quad spans no area draws
 * nothing, as in the native tile executor. */
export function buildSdfGlyphInstanceVertices(
  instances: GlyphInstance[],
  layerSlots: ReadonlyMap<string, number>,
  pageSize: SdfAtlasPageSize,
  clips: ReadonlyMap<string, ClipQuad> = new Map(),
  commandSlots: ReadonlyMap<string, number> = layerSlots
): Float32Array {
  const rows: number[] = [];
  for (const instance of instances) {
    if (!instance.drawable || instance.quad.length < 4) continue;
    const quad = instance.quad.slice(0, 4).map(([x, y]) => [f32(x), f32(y)] as [number, number]);
    const rect = atlasRect(instance.quad, pageSize);
    if (rect[2] <= 0 || rect[3] <= 0) continue;
    const toTexel = glyphTexelMap(quad, rect);
    if (!toTexel) continue;
    const clip = clips.get(instance.layerId) ?? NO_CLIP;
    rows.push(
      ...quad.flat(),
      ...toTexel,
      ...rect,
      ...instance.fill, ...instance.outline,
      instance.shaderFaceScale, instance.shaderFaceBias,
      instance.shaderUnderlayScale, instance.shaderUnderlayBias,
      instance.shaderVertexAlpha, layerSlots.get(instance.layerId) ?? 0,
      instance.atlasPage ?? 0, commandSlots.get(instance.layerId) ?? 0,
      ...clip.flat(),
    );
  }
  return new Float32Array(rows);
}

const f32 = Math.fround;
/** `mul_add`, rounded to double and then to single precision, which differs
 * from a single rounding only when the double lies exactly halfway between
 * two single-precision values. */
const fma = (a: number, b: number, c: number) => f32(a * b + c);

/** `pixel_sampling::TEXEL_CENTRE` of the shared core. */
const TEXEL_CENTRE = 0.5;
/** Largest quad determinant the native tile executor treats as no area. */
const DEGENERATE_QUAD_DETERMINANT = Math.fround(1e-6);

/** The glyph's atlas rectangle in texels, from the atlas coordinates of its
 * top-left and bottom-right corners. */
function atlasRect(quad: GlyphInstance["quad"], pageSize: SdfAtlasPageSize): [number, number, number, number] {
  const x = Math.round(quad[0][2] * pageSize.width);
  const y = Math.round(quad[0][3] * pageSize.height);
  return [x, y, Math.round(quad[2][2] * pageSize.width) - x, Math.round(quad[2][3] * pageSize.height) - y];
}

/** The canvas-to-texel matrix `[a, b, c, d, e, f]` of a glyph whose quad
 * (top-left, top-right, bottom-right, bottom-left) spans its atlas rectangle
 * edge to edge, evaluated in single precision in the native tile executor's
 * order of operations. Texel centres lie at whole texel coordinates, so the
 * quad's top-left corner maps to the rectangle's corner less the texel
 * centre. Null for a quad the native executor does not draw. */
export function glyphTexelMap(
  quad: ReadonlyArray<readonly [number, number]>,
  rect: readonly [number, number, number, number],
): [number, number, number, number, number, number] | null {
  const [topLeft, topRight, , bottomLeft] = quad;
  const ex = [f32(topRight[0] - topLeft[0]), f32(topRight[1] - topLeft[1])];
  const ey = [f32(bottomLeft[0] - topLeft[0]), f32(bottomLeft[1] - topLeft[1])];
  const determinant = f32(f32(ex[0] * ey[1]) - f32(ex[1] * ey[0]));
  if (!(Math.abs(determinant) > DEGENERATE_QUAD_DETERMINANT)) return null;
  const inverse = f32(1 / determinant);
  const aDx = f32(ey[1] * inverse);
  const aDy = f32(-ey[0] * inverse);
  const bDx = f32(-ex[1] * inverse);
  const bDy = f32(ex[0] * inverse);
  const aC = -f32(f32(aDx * topLeft[0]) + f32(aDy * topLeft[1]));
  const bC = -f32(f32(bDx * topLeft[0]) + f32(bDy * topLeft[1]));
  const [x, y, width, height] = rect.map(f32);
  return [
    f32(width * aDx),
    f32(height * bDx),
    f32(width * aDy),
    f32(height * bDy),
    fma(width, aC, f32(x - TEXEL_CENTRE)),
    fma(height, bC, f32(y - TEXEL_CENTRE)),
  ];
}

function instanceAttribute(gl: WebGL2RenderingContext, index: number, size: number, stride: number, offset: number): void {
  gl.enableVertexAttribArray(index);
  gl.vertexAttribPointer(index, size, gl.FLOAT, false, stride, offset);
  gl.vertexAttribDivisor(index, 1);
}

function createProgram(gl: WebGL2RenderingContext, vertexSource: string, fragmentSource: string): WebGLProgram {
  const compile = (type: number, source: string) => {
    const shader = gl.createShader(type);
    if (!shader) throw new Error("glyph shader allocation failed");
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(shader) ?? "glyph shader compile failed");
    return shader;
  };
  const vertex = compile(gl.VERTEX_SHADER, vertexSource);
  const fragment = compile(gl.FRAGMENT_SHADER, fragmentSource);
  const program = gl.createProgram();
  if (!program) throw new Error("glyph program allocation failed");
  gl.attachShader(program, vertex);
  gl.attachShader(program, fragment);
  gl.linkProgram(program);
  gl.deleteShader(vertex);
  gl.deleteShader(fragment);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program) ?? "glyph program link failed");
  return program;
}

const VERTEX_SHADER = `#version 300 es
precision highp float;
layout(location=0) in vec4 a_quad01;
layout(location=1) in vec4 a_quad23;
layout(location=2) in vec4 a_toTexel;
layout(location=3) in vec2 a_toTexelOffset;
layout(location=4) in vec4 a_rect;
layout(location=5) in vec4 a_color;
layout(location=6) in vec4 a_outline;
layout(location=7) in vec4 a_sdfParams;
layout(location=8) in vec4 a_instanceMeta;
layout(location=9) in vec4 a_clip01;
layout(location=10) in vec4 a_clip23;
uniform sampler2D u_layerState;
uniform highp usampler2D u_renderMask;
uniform float u_layerStateWidth;
uniform highp usampler2D u_commandMask;
uniform sampler2D u_commandState;
uniform float u_commandWidth;
uniform sampler2D u_previewTransform;
flat out vec4 v_toTexel;
flat out vec2 v_toTexelOffset;
flat out vec4 v_quad01;
flat out vec4 v_quad23;
flat out vec4 v_rect;
flat out vec4 v_color;
flat out vec4 v_outline;
flat out vec4 v_sdfParams;
flat out float v_vertexAlpha;
flat out int v_atlasPage;
flat out vec4 v_clip01;
flat out vec4 v_clip23;
// Device pixels the quad is grown by on every side; the fragment stage
// decides which pixels the glyph covers.
const float QUAD_OUTSET = 0.5;
// pixel_sampling::TEXEL_CENTRE of the shared core.
const float TEXEL_CENTRE = 0.5;
// sdf_material::TMP_MIN_SHADER_SCALE of the shared core.
const float MIN_SHADER_SCALE = 0.0001;
const int CORNER_IDS[6] = int[6](0, 1, 3, 3, 1, 2);
vec2 previewed(vec2 point, vec4 preview0, vec4 preview1) {
  return vec2(dot(preview0.xy, point) + preview0.z, dot(preview1.xy, point) + preview1.z);
}
void main() {
  float layerSlot = a_instanceMeta.y;
  float stateU = (layerSlot + 0.5) / max(u_layerStateWidth, 1.0);
  vec2 state = texture(u_layerState, vec2(stateU, 0.5)).rg;
  uint mask = texelFetch(u_renderMask, ivec2(int(layerSlot), 0), 0).r;
  uint commandMask = texelFetch(u_commandMask, ivec2(int(a_instanceMeta.w), 0), 0).r;
  vec2 commandState = texelFetch(u_commandState, ivec2(int(a_instanceMeta.w), 0), 0).rg;
  vec2 totalState = state + commandState;
  vec4 preview0 = texelFetch(u_previewTransform, ivec2(int(layerSlot), 0), 0);
  vec4 preview1 = texelFetch(u_previewTransform, ivec2(int(layerSlot), 1), 0);
  // The canvas-to-texel matrix after the dynamic offset and the preview: the
  // preview is undone, then the offset, then the glyph's own mapping.
  float previewDeterminant = preview0.x * preview1.y - preview1.x * preview0.y;
  vec4 previewInverse = vec4(preview1.y, -preview1.x, -preview0.y, preview0.x) / previewDeterminant;
  vec2 previewInverseOffset = vec2(
    preview0.y * preview1.z - preview1.y * preview0.z,
    preview1.x * preview0.z - preview0.x * preview1.z
  ) / previewDeterminant;
  vec2 undone = previewInverseOffset - totalState;
  vec4 toTexel = vec4(
    a_toTexel.x * previewInverse.x + a_toTexel.z * previewInverse.y,
    a_toTexel.y * previewInverse.x + a_toTexel.w * previewInverse.y,
    a_toTexel.x * previewInverse.z + a_toTexel.z * previewInverse.w,
    a_toTexel.y * previewInverse.z + a_toTexel.w * previewInverse.w
  );
  vec2 toTexelOffset = vec2(
    a_toTexel.x * undone.x + a_toTexel.z * undone.y + a_toTexelOffset.x,
    a_toTexel.y * undone.x + a_toTexel.w * undone.y + a_toTexelOffset.y
  );
  // Texels to canvas, for the drawn quad's corners. The glyph spans its
  // atlas rectangle edge to edge; a canvas step of QUAD_OUTSET pixels moves a
  // texel coordinate by at most QUAD_OUTSET times the length of its row of
  // the matrix, so the rectangle grown by that much reaches every pixel
  // within QUAD_OUTSET of the glyph.
  float determinant = toTexel.x * toTexel.w - toTexel.y * toTexel.z;
  vec4 forward = vec4(toTexel.w, -toTexel.y, -toTexel.z, toTexel.x) / determinant;
  vec2 forwardOffset = vec2(
    toTexel.z * toTexelOffset.y - toTexel.w * toTexelOffset.x,
    toTexel.y * toTexelOffset.x - toTexel.x * toTexelOffset.y
  ) / determinant;
  int corner = CORNER_IDS[gl_VertexID];
  vec2 unit = vec2(corner == 1 || corner == 2 ? 1.0 : 0.0, corner >= 2 ? 1.0 : 0.0);
  vec2 outset = QUAD_OUTSET * vec2(length(toTexel.xz), length(toTexel.yw));
  vec2 texel = a_rect.xy - TEXEL_CENTRE + unit * a_rect.zw + (unit * 2.0 - 1.0) * outset;
  vec2 point = vec2(
    forward.x * texel.x + forward.z * texel.y + forwardOffset.x,
    forward.y * texel.x + forward.w * texel.y + forwardOffset.y
  );
  vec2 position = vec2(point.x * ${2 / CARD_W} - 1.0, 1.0 - point.y * ${2 / CARD_H});
  if (mask == 0u || commandMask == 0u) position = vec2(2.0);
  gl_Position = vec4(position, 0.0, 1.0);
  v_toTexel = toTexel;
  v_toTexelOffset = toTexelOffset;
  v_quad01 = vec4(previewed(a_quad01.xy + totalState, preview0, preview1), previewed(a_quad01.zw + totalState, preview0, preview1));
  v_quad23 = vec4(previewed(a_quad23.xy + totalState, preview0, preview1), previewed(a_quad23.zw + totalState, preview0, preview1));
  v_rect = a_rect;
  v_color = a_color;
  v_outline = a_outline;
  v_sdfParams = vec4(max(a_sdfParams.x, MIN_SHADER_SCALE), a_sdfParams.y, max(a_sdfParams.z, MIN_SHADER_SCALE), a_sdfParams.w);
  v_vertexAlpha = clamp(a_instanceMeta.x, 0.0, 1.0);
  v_atlasPage = int(a_instanceMeta.z);
  v_clip01 = vec4(previewed(a_clip01.xy + state, preview0, preview1), previewed(a_clip01.zw + state, preview0, preview1));
  v_clip23 = vec4(previewed(a_clip23.xy + state, preview0, preview1), previewed(a_clip23.zw + state, preview0, preview1));
}`;

// The glyph at the pixel under the fragment, as the native tile executor
// shades it: the pixel is drawn when its centre lies in the clip and in the
// glyph quad, and the distance field is filtered bilinearly at the centre's
// texel position. Sampling positions are derived from the fragment position
// so they do not depend on the rasteriser's vertex precision.
const FRAGMENT_SHADER = `#version 300 es
precision highp float;
precision highp int;
uniform highp sampler2DArray u_atlas;
flat in vec4 v_toTexel;
flat in vec2 v_toTexelOffset;
flat in vec4 v_quad01;
flat in vec4 v_quad23;
flat in vec4 v_rect;
flat in vec4 v_color;
flat in vec4 v_outline;
flat in vec4 v_sdfParams;
flat in float v_vertexAlpha;
flat in int v_atlasPage;
flat in vec4 v_clip01;
flat in vec4 v_clip23;
out vec4 outColor;
${COMMAND_CLIP_GLSL}
// Top-left corner of the canvas pixel under the fragment, +y down.
vec2 canvasPixel() {
  vec2 window = floor(gl_FragCoord.xy);
  return vec2(window.x, ${CARD_H - 1}.0 - window.y);
}
// Whether the glyph quad covers a canvas point: on the point's row the sides
// that cross its height bound a span, which holds [min, max), as the native
// tile executor scans a quad.
bool quadCovers(vec2 point) {
  highp vec2 p[4];
  p[0] = v_quad01.xy;
  p[1] = v_quad01.zw;
  p[2] = v_quad23.xy;
  p[3] = v_quad23.zw;
  float low = 0.0;
  float high = 0.0;
  int crossings = 0;
  for (int side = 0; side < 4; side += 1) {
    vec2 start = p[side];
    vec2 end = p[(side + 1) % 4];
    float bottom = min(start.y, end.y);
    float top = max(start.y, end.y);
    if (point.y < bottom || point.y >= top || top == bottom) continue;
    float t = (point.y - start.y) / (end.y - start.y);
    float x = (end.x - start.x) * t + start.x;
    low = crossings == 0 ? x : min(low, x);
    high = crossings == 0 ? x : max(high, x);
    crossings += 1;
  }
  return crossings >= 2 && point.x >= low && point.x < high;
}
float atlasTexel(ivec2 texel) {
  return texelFetch(u_atlas, ivec3(texel, v_atlasPage), 0).r;
}
void main() {
  vec2 centre = canvasPixel() + vec2(0.5);
  if (!insideClip(centre) || !quadCovers(centre)) discard;
  vec2 texel = vec2(
    v_toTexel.x * centre.x + (v_toTexel.z * centre.y + v_toTexelOffset.x),
    v_toTexel.y * centre.x + (v_toTexel.w * centre.y + v_toTexelOffset.y)
  );
  // The four texels whose centres surround the sample, clamped to the
  // glyph's rectangle, weighed by the sample's position between them.
  vec2 base = floor(texel);
  vec2 weight = texel - base;
  ivec2 first = ivec2(v_rect.xy);
  ivec2 last = first + ivec2(v_rect.zw) - 1;
  ivec2 low = clamp(ivec2(base), first, last);
  ivec2 high = clamp(ivec2(base) + 1, first, last);
  float p00 = atlasTexel(low);
  float p10 = atlasTexel(ivec2(high.x, low.y));
  float p01 = atlasTexel(ivec2(low.x, high.y));
  float p11 = atlasTexel(high);
  float top = (p10 - p00) * weight.x + p00;
  float bottom = (p11 - p01) * weight.x + p01;
  float sdf = (bottom - top) * weight.y + top;
  float faceT = clamp(sdf * v_sdfParams.x - v_sdfParams.y, 0.0, 1.0);
  float underlayT = clamp(sdf * v_sdfParams.z - v_sdfParams.w, 0.0, 1.0);
  vec4 face = vec4(v_color.rgb * v_color.a, v_color.a);
  vec4 outline = vec4(v_outline.rgb * v_outline.a, v_outline.a);
  float outlineWeight = underlayT * (1.0 - face.a * faceT);
  outColor = (outline * outlineWeight + face * faceT) * v_vertexAlpha;
}`;
