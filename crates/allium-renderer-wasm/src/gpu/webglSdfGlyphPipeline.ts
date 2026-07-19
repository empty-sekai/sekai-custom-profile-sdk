import type { GlyphInstance } from "../types/glyph.js";

const CARD_W = 1830;
const CARD_H = 812;
const FLOATS_PER_INSTANCE = 36;

type GlyphGpuBatch = {
  vao: WebGLVertexArrayObject;
  buffer: WebGLBuffer;
  instances: number;
  bytes: number;
};

export class WebglSdfGlyphPipeline {
  private readonly gl: WebGL2RenderingContext;
  private readonly program: WebGLProgram;
  private readonly batches = new Map<string, GlyphGpuBatch>();
  private atlasTexture: WebGLTexture;
  private ownsAtlas = true;
  private geometryBuilds = 0;

  constructor(gl: WebGL2RenderingContext) {
    this.gl = gl;
    this.program = createProgram(gl, VERTEX_SHADER, FRAGMENT_SHADER);
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
    if (vertices.length % FLOATS_PER_INSTANCE !== 0) throw new Error(`invalid glyph instance buffer ${key}`);
    const gl = this.gl;
    const vao = gl.createVertexArray();
    const buffer = gl.createBuffer();
    if (!vao || !buffer) throw new Error("glyph pipeline batch allocation failed");
    gl.bindVertexArray(vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(gl.ARRAY_BUFFER, vertices, gl.STATIC_DRAW);
    const stride = FLOATS_PER_INSTANCE * 4;
    instanceAttribute(gl, 0, 4, stride, 0);
    instanceAttribute(gl, 1, 4, stride, 4 * 4);
    instanceAttribute(gl, 2, 4, stride, 8 * 4);
    instanceAttribute(gl, 3, 4, stride, 12 * 4);
    instanceAttribute(gl, 4, 4, stride, 16 * 4);
    instanceAttribute(gl, 5, 4, stride, 20 * 4);
    instanceAttribute(gl, 6, 4, stride, 24 * 4);
    instanceAttribute(gl, 7, 4, stride, 28 * 4);
    instanceAttribute(gl, 8, 4, stride, 32 * 4);
    gl.bindVertexArray(null);
    this.batches.set(key, { vao, buffer, instances: vertices.length / FLOATS_PER_INSTANCE, bytes: vertices.byteLength });
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
    gl.useProgram(this.program);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D_ARRAY, this.atlasTexture);
    gl.uniform1i(gl.getUniformLocation(this.program, "u_atlas"), 0);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, stateTexture);
    gl.uniform1i(gl.getUniformLocation(this.program, "u_layerState"), 1);
    gl.uniform1f(gl.getUniformLocation(this.program, "u_layerStateWidth"), stateWidth);
    gl.activeTexture(gl.TEXTURE2);
    gl.bindTexture(gl.TEXTURE_2D, maskTexture);
    gl.uniform1i(gl.getUniformLocation(this.program, "u_renderMask"), 2);
    gl.activeTexture(gl.TEXTURE3);
    gl.bindTexture(gl.TEXTURE_2D, commandMaskTexture);
    gl.uniform1i(gl.getUniformLocation(this.program, "u_commandMask"), 3);
    gl.activeTexture(gl.TEXTURE4);
    gl.bindTexture(gl.TEXTURE_2D, commandStateTexture);
    gl.uniform1i(gl.getUniformLocation(this.program, "u_commandState"), 4);
    gl.uniform1f(gl.getUniformLocation(this.program, "u_commandWidth"), commandWidth);
    gl.activeTexture(gl.TEXTURE5);
    gl.bindTexture(gl.TEXTURE_2D, previewTransformTexture);
    gl.uniform1i(gl.getUniformLocation(this.program, "u_previewTransform"), 5);
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

export function buildSdfGlyphInstanceVertices(
  instances: GlyphInstance[],
  layerSlots: ReadonlyMap<string, number>,
  clips: ReadonlyMap<string, [[number, number], [number, number], [number, number], [number, number]]> = new Map(),
  commandSlots: ReadonlyMap<string, number> = layerSlots
): Float32Array {
  const rows: number[] = [];
  for (const instance of instances) {
    if (!instance.drawable || instance.quad.length < 4) continue;
    const [tl, tr, br, bl] = instance.quad;
    const position = ([x, y]: number[]) => [(x / CARD_W) * 2 - 1, 1 - (y / CARD_H) * 2];
    const clip = clips.get(instance.layerId) ?? [[-1e9, -1e9], [1e9, -1e9], [1e9, 1e9], [-1e9, 1e9]];
    rows.push(
      ...position(tl), ...position(tr), ...position(br), ...position(bl),
      tl[2], tl[3], br[2], br[3], ...instance.fill, ...instance.outline,
      instance.shaderFaceScale, instance.shaderFaceBias,
      instance.shaderUnderlayScale, instance.shaderUnderlayBias,
      instance.shaderVertexAlpha, layerSlots.get(instance.layerId) ?? 0,
      instance.atlasPage ?? 0, commandSlots.get(instance.layerId) ?? 0,
      ...position(clip[0]), ...position(clip[1]), ...position(clip[2]), ...position(clip[3]),
    );
  }
  return new Float32Array(rows);
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
layout(location=0) in vec4 a_pos01;
layout(location=1) in vec4 a_pos23;
layout(location=2) in vec4 a_uvRect;
layout(location=3) in vec4 a_color;
layout(location=4) in vec4 a_outline;
layout(location=5) in vec4 a_sdfParams;
layout(location=6) in vec4 a_instanceMeta;
layout(location=7) in vec4 a_clip01;
layout(location=8) in vec4 a_clip23;
uniform sampler2D u_layerState;
uniform highp usampler2D u_renderMask;
uniform float u_layerStateWidth;
uniform highp usampler2D u_commandMask;
uniform sampler2D u_commandState;
uniform float u_commandWidth;
uniform sampler2D u_previewTransform;
out vec2 v_uv;
out vec4 v_color;
out vec4 v_outline;
out float v_faceScale;
out float v_faceBias;
out float v_underlayScale;
out float v_underlayBias;
out float v_vertexAlpha;
flat out float v_atlasPage;
out vec2 v_point;
out vec4 v_clip01;
out vec4 v_clip23;
const int CORNER_IDS[6] = int[6](0, 1, 3, 3, 1, 2);
void main() {
  vec2 positions[4] = vec2[4](a_pos01.xy, a_pos01.zw, a_pos23.xy, a_pos23.zw);
  vec2 uvs[4] = vec2[4](a_uvRect.xy, vec2(a_uvRect.z, a_uvRect.y), a_uvRect.zw, vec2(a_uvRect.x, a_uvRect.w));
  int corner = CORNER_IDS[gl_VertexID];
  float layerSlot = a_instanceMeta.y;
  float stateU = (layerSlot + 0.5) / max(u_layerStateWidth, 1.0);
  vec2 state = texture(u_layerState, vec2(stateU, 0.5)).rg;
  uint mask = texelFetch(u_renderMask, ivec2(int(layerSlot), 0), 0).r;
  uint commandMask = texelFetch(u_commandMask, ivec2(int(a_instanceMeta.w), 0), 0).r;
  vec2 commandState = texelFetch(u_commandState, ivec2(int(a_instanceMeta.w), 0), 0).rg;
  vec2 totalState = state + commandState;
  vec4 preview0 = texelFetch(u_previewTransform, ivec2(int(layerSlot), 0), 0);
  vec4 preview1 = texelFetch(u_previewTransform, ivec2(int(layerSlot), 1), 0);
  vec2 pixelPosition = vec2(
    (positions[corner].x + 1.0) * ${CARD_W / 2}.0,
    (1.0 - positions[corner].y) * ${CARD_H / 2}.0
  ) + totalState;
  vec2 previewedPosition = vec2(
    dot(preview0.xy, pixelPosition) + preview0.z,
    dot(preview1.xy, pixelPosition) + preview1.z
  );
  vec2 position = vec2(
    previewedPosition.x * ${2 / CARD_W} - 1.0,
    1.0 - previewedPosition.y * ${2 / CARD_H}
  );
  vec2 clipPoints[4] = vec2[4](a_clip01.xy, a_clip01.zw, a_clip23.xy, a_clip23.zw);
  for (int index = 0; index < 4; index += 1) {
    vec2 clipPixel = vec2((clipPoints[index].x + 1.0) * ${CARD_W / 2}.0, (1.0 - clipPoints[index].y) * ${CARD_H / 2}.0) + state;
    vec2 transformed = vec2(dot(preview0.xy, clipPixel) + preview0.z, dot(preview1.xy, clipPixel) + preview1.z);
    clipPoints[index] = vec2(transformed.x * ${2 / CARD_W} - 1.0, 1.0 - transformed.y * ${2 / CARD_H});
  }
  if (mask == 0u || commandMask == 0u) position = vec2(2.0);
  gl_Position = vec4(position, 0.0, 1.0);
  v_uv = uvs[corner];
  v_color = a_color;
  v_outline = a_outline;
  v_faceScale = max(a_sdfParams.x, 0.0001);
  v_faceBias = a_sdfParams.y;
  v_underlayScale = max(a_sdfParams.z, 0.0001);
  v_underlayBias = a_sdfParams.w;
  v_vertexAlpha = clamp(a_instanceMeta.x, 0.0, 1.0);
  v_atlasPage = a_instanceMeta.z;
  v_point = position;
  v_clip01 = vec4(clipPoints[0], clipPoints[1]);
  v_clip23 = vec4(clipPoints[2], clipPoints[3]);
}`;

const FRAGMENT_SHADER = `#version 300 es
precision highp float;
uniform highp sampler2DArray u_atlas;
in vec2 v_uv;
in vec4 v_color;
in vec4 v_outline;
in float v_faceScale;
in float v_faceBias;
in float v_underlayScale;
in float v_underlayBias;
in float v_vertexAlpha;
flat in float v_atlasPage;
in vec2 v_point;
in vec4 v_clip01;
in vec4 v_clip23;
out vec4 outColor;
float cross2(vec2 a, vec2 b) { return a.x * b.y - a.y * b.x; }
bool insideClip() {
  vec2 p[4] = vec2[4](v_clip01.xy, v_clip01.zw, v_clip23.xy, v_clip23.zw);
  float c0 = cross2(p[1] - p[0], v_point - p[0]);
  float c1 = cross2(p[2] - p[1], v_point - p[1]);
  float c2 = cross2(p[3] - p[2], v_point - p[2]);
  float c3 = cross2(p[0] - p[3], v_point - p[3]);
  return (c0 >= 0.0 && c1 >= 0.0 && c2 >= 0.0 && c3 >= 0.0) || (c0 <= 0.0 && c1 <= 0.0 && c2 <= 0.0 && c3 <= 0.0);
}
void main() {
  if (!insideClip()) discard;
  float sdf = texture(u_atlas, vec3(v_uv, v_atlasPage)).r;
  float faceT = clamp(sdf * v_faceScale - v_faceBias, 0.0, 1.0);
  float underlayT = clamp(sdf * v_underlayScale - v_underlayBias, 0.0, 1.0) * clamp(sdf * 12.5, 0.0, 1.0);
  vec4 face = vec4(v_color.rgb * v_color.a, v_color.a);
  vec4 outline = vec4(v_outline.rgb * v_outline.a, v_outline.a);
  float oneMinusFace = 1.0 - face.a * faceT;
  outColor = (face * faceT + outline * underlayT * oneMinusFace) * v_vertexAlpha;
}`;
