import { SEMANTIC_FLOATS_PER_VERTEX, semanticTextBatchKey, type SemanticDrawBatch } from "./semanticCommandGeometry.js";
import type { SemanticCommandPlan, SemanticCommandStatePatch, SemanticLayerPatch } from "./semanticCommandPlanner.js";
import { WebglSdfGlyphPipeline } from "./webglSdfGlyphPipeline.js";
import { WebglSdfAtlasTexture } from "./webglSdfAtlasTexture.js";
import type { SdfAtlas } from "../fontSdfAtlas.js";

const CARD_W = 1830;
const CARD_H = 812;

type GpuBatch = {
  source: SemanticDrawBatch;
  vao: WebGLVertexArrayObject;
  vertexBuffer: WebGLBuffer;
  slotBuffer: WebGLBuffer;
  commandSlotBuffer: WebGLBuffer;
  vertices: number;
};

export type SemanticGpuMetrics = {
  drawCalls: number;
  geometryBuilds: number;
  vertexBytes: number;
  textureUploads: number;
  textureBytes: number;
  stateUploadBytes: number;
  maskUploadBytes: number;
  glyphGeometryBuilds: number;
};

export class WebglSemanticCommandExecutor {
  private shapeProgram: WebGLProgram;
  private textureProgram: WebGLProgram;
  private stateTexture: WebGLTexture;
  private maskTexture: WebGLTexture;
  private commandMaskTexture: WebGLTexture;
  private commandStateTexture: WebGLTexture;
  private batches: GpuBatch[] = [];
  private textures = new Map<string, { texture: WebGLTexture; source: TexImageSource; bytes: number }>();
  private state = new Float32Array(2);
  private mask = new Uint8Array(1);
  private stateWidth = 1;
  private commandMask = new Uint8Array(1);
  private commandState = new Float32Array(2);
  private commandWidth = 1;
  private plan: SemanticCommandPlan | null = null;
  private readonly glyphPipeline: WebglSdfGlyphPipeline;
  private sdfAtlasTexture: WebglSdfAtlasTexture | null = null;
  private geometryBuilds = 0;
  private textureUploads = 0;
  private textureBytes = 0;

  constructor(private readonly gl: WebGL2RenderingContext, private readonly canvasWidth = CARD_W, private readonly canvasHeight = CARD_H) {
    this.shapeProgram = createProgram(gl, VERTEX_SHADER, SHAPE_FRAGMENT_SHADER);
    this.textureProgram = createProgram(gl, VERTEX_SHADER, TEXTURE_FRAGMENT_SHADER);
    const stateTexture = gl.createTexture();
    const maskTexture = gl.createTexture();
    const commandMaskTexture = gl.createTexture();
    const commandStateTexture = gl.createTexture();
    if (!stateTexture || !maskTexture || !commandMaskTexture || !commandStateTexture) throw new Error("semantic WebGL state texture creation failed");
    this.stateTexture = stateTexture;
    this.maskTexture = maskTexture;
    this.commandMaskTexture = commandMaskTexture;
    this.commandStateTexture = commandStateTexture;
    this.glyphPipeline = new WebglSdfGlyphPipeline(gl);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
  }

  setTextGlyphBatch(commandId: string, vertices: Float32Array): void {
    this.glyphPipeline.upload(commandId, vertices);
  }

  async setSdfAtlas(atlas: SdfAtlas): Promise<{ bytes: number; rects: number }> {
    this.sdfAtlasTexture ??= new WebglSdfAtlasTexture(this.gl);
    const uploaded = await this.sdfAtlasTexture.uploadUpdates(atlas);
    this.glyphPipeline.setAtlas(this.sdfAtlasTexture.texture());
    return uploaded;
  }

  setScene(plan: SemanticCommandPlan, batches: SemanticDrawBatch[], resources: Map<string, TexImageSource>): void {
    this.deleteBatches();
    this.glyphPipeline.clearBatches();
    this.plan = plan;
    const operations = plan.operations();
    this.stateWidth = Math.max(1, ...operations.map((operation) => operation.layerSlot + 1));
    this.state = new Float32Array(this.stateWidth * 2);
    this.mask = new Uint8Array(this.stateWidth);
    this.commandWidth = Math.max(1, ...operations.map((operation) => operation.commandSlot + 1));
    this.commandMask = new Uint8Array(this.commandWidth);
    this.commandState = new Float32Array(this.commandWidth * 2);
    const initialized = new Set<number>();
    for (const operation of operations) {
      this.commandMask[operation.commandSlot] = operation.commandVisible ? 1 : 0;
      this.commandState[operation.commandSlot * 2] = operation.commandTransform.dx;
      this.commandState[operation.commandSlot * 2 + 1] = operation.commandTransform.dy;
      const slot = operation.layerSlot;
      if (initialized.has(slot)) continue;
      initialized.add(slot);
      this.state[slot * 2] = operation.transform.dx;
      this.state[slot * 2 + 1] = operation.transform.dy;
      this.mask[slot] = operation.visible ? 1 : 0;
    }
    this.uploadFullState();
    for (const batch of batches) this.batches.push(this.uploadBatch(batch));
    this.geometryBuilds += 1;
    for (const [key, source] of resources) this.ensureTexture(key, source);
  }

  applyCommandPatches(patches: SemanticCommandStatePatch[]): { commandMaskUploadBytes: number; commandStateUploadBytes: number } {
    if (!this.plan) throw new Error("semantic GPU scene is not initialized");
    this.plan.applyCommandPatches(patches);
    let commandMaskUploadBytes = 0;
    let commandStateUploadBytes = 0;
    for (const patch of patches) {
      if (!Number.isInteger(patch.slot) || patch.slot < 0 || patch.slot >= this.commandWidth) throw new Error(`invalid command slot ${patch.slot}`);
      if (patch.transform != null) {
        const offset = patch.slot * 2;
        this.commandState[offset] = patch.transform.dx;
        this.commandState[offset + 1] = patch.transform.dy;
        this.gl.bindTexture(this.gl.TEXTURE_2D, this.commandStateTexture);
        this.gl.texSubImage2D(this.gl.TEXTURE_2D, 0, patch.slot, 0, 1, 1, this.gl.RG, this.gl.FLOAT, this.commandState.subarray(offset, offset + 2));
        commandStateUploadBytes += 8;
      }
      if (patch.render_mask != null) {
        this.commandMask[patch.slot] = patch.render_mask ? 1 : 0;
        this.gl.bindTexture(this.gl.TEXTURE_2D, this.commandMaskTexture);
        this.gl.texSubImage2D(this.gl.TEXTURE_2D, 0, patch.slot, 0, 1, 1, this.gl.RED_INTEGER, this.gl.UNSIGNED_BYTE, this.commandMask.subarray(patch.slot, patch.slot + 1));
        commandMaskUploadBytes += 1;
      }
    }
    return { commandMaskUploadBytes, commandStateUploadBytes };
  }

  applyLayerPatches(patches: SemanticLayerPatch[]): { stateUploadBytes: number; maskUploadBytes: number } {
    if (!this.plan) throw new Error("semantic GPU scene is not initialized");
    this.plan.applyLayerPatches(patches);
    const slotByLayer = new Map(this.plan.operations().map((operation) => [operation.layerId, operation.layerSlot] as const));
    let stateUploadBytes = 0;
    let maskUploadBytes = 0;
    for (const patch of patches) {
      const slot = slotByLayer.get(patch.layer_id);
      if (slot == null) throw new Error(`semantic GPU patch references unknown layer ${patch.layer_id}`);
      if (patch.transform) {
        const offset = slot * 2;
        this.state[offset] = patch.transform.dx;
        this.state[offset + 1] = patch.transform.dy;
        this.gl.bindTexture(this.gl.TEXTURE_2D, this.stateTexture);
        this.gl.texSubImage2D(this.gl.TEXTURE_2D, 0, slot, 0, 1, 1, this.gl.RG, this.gl.FLOAT, this.state.subarray(offset, offset + 2));
        stateUploadBytes += 8;
      }
      if (patch.render_mask != null) {
        this.mask[slot] = patch.render_mask ? 1 : 0;
        this.gl.bindTexture(this.gl.TEXTURE_2D, this.maskTexture);
        this.gl.texSubImage2D(this.gl.TEXTURE_2D, 0, slot, 0, 1, 1, this.gl.RED_INTEGER, this.gl.UNSIGNED_BYTE, this.mask.subarray(slot, slot + 1));
        maskUploadBytes += 1;
      }
    }
    return { stateUploadBytes, maskUploadBytes };
  }

  draw(): SemanticGpuMetrics {
    const gl = this.gl;
    let drawCalls = 0;
    let vertexBytes = 0;
    for (const batch of this.batches) {
      if (batch.source.kind === "composite") continue;
      if (batch.source.kind === "text") {
        const glyph = this.glyphPipeline.draw(semanticTextBatchKey(batch.source.commandIds), this.stateTexture, this.maskTexture, this.stateWidth, this.commandMaskTexture, this.commandStateTexture, this.commandWidth);
        drawCalls += glyph.drawCalls;
        vertexBytes += glyph.bytes;
        continue;
      }
      const program = batch.source.kind === "shape" ? this.shapeProgram : this.textureProgram;
      gl.useProgram(program);
      this.bindCommon(program);
      if (batch.source.kind !== "shape") {
        const resource = batch.source.resource;
        if (!resource) throw new Error(`semantic ${batch.source.kind} batch has no resource`);
        const key = resourceIdentity(resource.namespace, resource.key);
        const texture = this.textures.get(key)?.texture;
        if (!texture) throw new Error(`semantic GPU resource not loaded ${key}`);
        gl.activeTexture(gl.TEXTURE2);
        gl.bindTexture(gl.TEXTURE_2D, texture);
        const filter = gl.NEAREST;
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter);
        gl.uniform1i(gl.getUniformLocation(program, "u_image"), 2);
        gl.uniform1i(gl.getUniformLocation(program, "u_maskMode"), batch.source.kind === "mask" ? 1 : 0);
        const alphaMask = batch.source.maskResource
          ? this.textures.get(resourceIdentity(batch.source.maskResource.namespace, batch.source.maskResource.key))?.texture ?? null
          : null;
        gl.activeTexture(gl.TEXTURE5);
        gl.bindTexture(gl.TEXTURE_2D, alphaMask);
        gl.uniform1i(gl.getUniformLocation(program, "u_alphaMask"), 5);
        gl.uniform1i(gl.getUniformLocation(program, "u_hasAlphaMask"), alphaMask ? 1 : 0);
      }
      gl.bindVertexArray(batch.vao);
      gl.drawArrays(gl.TRIANGLES, 0, batch.vertices);
      drawCalls += 1;
      vertexBytes += batch.source.vertices.byteLength + batch.source.layerSlots.byteLength + batch.source.commandSlots.byteLength;
    }
    gl.bindVertexArray(null);
    return {
      drawCalls,
      geometryBuilds: this.geometryBuilds,
      vertexBytes,
      textureUploads: this.textureUploads,
      textureBytes: this.textureBytes,
      stateUploadBytes: 0,
      maskUploadBytes: 0,
      glyphGeometryBuilds: this.glyphPipeline.stats().geometryBuilds,
    };
  }

  destroy(): void {
    this.deleteBatches();
    for (const entry of this.textures.values()) this.gl.deleteTexture(entry.texture);
    this.textures.clear();
    this.gl.deleteTexture(this.stateTexture);
    this.gl.deleteTexture(this.maskTexture);
    this.gl.deleteTexture(this.commandMaskTexture);
    this.gl.deleteTexture(this.commandStateTexture);
    this.gl.deleteProgram(this.shapeProgram);
    this.gl.deleteProgram(this.textureProgram);
    this.glyphPipeline.destroy();
    this.sdfAtlasTexture?.destroy();
  }

  private bindCommon(program: WebGLProgram): void {
    const gl = this.gl;
    gl.uniform2f(gl.getUniformLocation(program, "u_canvas"), this.canvasWidth, this.canvasHeight);
    gl.uniform1f(gl.getUniformLocation(program, "u_stateWidth"), this.stateWidth);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.stateTexture);
    gl.uniform1i(gl.getUniformLocation(program, "u_state"), 0);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, this.maskTexture);
    gl.uniform1i(gl.getUniformLocation(program, "u_mask"), 1);
    gl.activeTexture(gl.TEXTURE3);
    gl.bindTexture(gl.TEXTURE_2D, this.commandMaskTexture);
    gl.uniform1i(gl.getUniformLocation(program, "u_commandMask"), 3);
    gl.activeTexture(gl.TEXTURE4);
    gl.bindTexture(gl.TEXTURE_2D, this.commandStateTexture);
    gl.uniform1i(gl.getUniformLocation(program, "u_commandState"), 4);
    gl.uniform1f(gl.getUniformLocation(program, "u_commandWidth"), this.commandWidth);
  }

  private uploadFullState(): void {
    const gl = this.gl;
    gl.bindTexture(gl.TEXTURE_2D, this.stateTexture);
    setStateTextureParameters(gl);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RG32F, this.stateWidth, 1, 0, gl.RG, gl.FLOAT, this.state);
    gl.bindTexture(gl.TEXTURE_2D, this.maskTexture);
    setStateTextureParameters(gl);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8UI, this.stateWidth, 1, 0, gl.RED_INTEGER, gl.UNSIGNED_BYTE, this.mask);
    gl.bindTexture(gl.TEXTURE_2D, this.commandMaskTexture);
    setStateTextureParameters(gl);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8UI, this.commandWidth, 1, 0, gl.RED_INTEGER, gl.UNSIGNED_BYTE, this.commandMask);
    gl.bindTexture(gl.TEXTURE_2D, this.commandStateTexture);
    setStateTextureParameters(gl);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RG32F, this.commandWidth, 1, 0, gl.RG, gl.FLOAT, this.commandState);
  }

  private uploadBatch(source: SemanticDrawBatch): GpuBatch {
    const gl = this.gl;
    const vao = gl.createVertexArray();
    const vertexBuffer = gl.createBuffer();
    const slotBuffer = gl.createBuffer();
    const commandSlotBuffer = gl.createBuffer();
    if (!vao || !vertexBuffer || !slotBuffer || !commandSlotBuffer) throw new Error("semantic WebGL batch allocation failed");
    gl.bindVertexArray(vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, vertexBuffer);
    gl.bufferData(gl.ARRAY_BUFFER, source.vertices, gl.STATIC_DRAW);
    const stride = SEMANTIC_FLOATS_PER_VERTEX * 4;
    floatAttribute(gl, 0, 2, stride, 0);
    floatAttribute(gl, 1, 2, stride, 2 * 4);
    floatAttribute(gl, 2, 2, stride, 4 * 4);
    floatAttribute(gl, 3, 4, stride, 6 * 4);
    floatAttribute(gl, 4, 4, stride, 10 * 4);
    floatAttribute(gl, 5, 4, stride, 14 * 4);
    floatAttribute(gl, 7, 4, stride, 18 * 4);
    floatAttribute(gl, 8, 4, stride, 22 * 4);
    gl.bindBuffer(gl.ARRAY_BUFFER, slotBuffer);
    gl.bufferData(gl.ARRAY_BUFFER, source.layerSlots, gl.STATIC_DRAW);
    gl.enableVertexAttribArray(6);
    gl.vertexAttribIPointer(6, 1, gl.UNSIGNED_INT, 4, 0);
    gl.bindBuffer(gl.ARRAY_BUFFER, commandSlotBuffer);
    gl.bufferData(gl.ARRAY_BUFFER, source.commandSlots, gl.STATIC_DRAW);
    gl.enableVertexAttribArray(9);
    gl.vertexAttribIPointer(9, 1, gl.UNSIGNED_INT, 4, 0);
    gl.bindVertexArray(null);
    return { source, vao, vertexBuffer, slotBuffer, commandSlotBuffer, vertices: source.layerSlots.length };
  }

  private ensureTexture(key: string, source: TexImageSource): void {
    if (this.textures.has(key)) return;
    const gl = this.gl;
    const texture = gl.createTexture();
    if (!texture) throw new Error(`semantic texture allocation failed ${key}`);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, 0);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, source);
    const width = "width" in source ? Number(source.width) : 0;
    const height = "height" in source ? Number(source.height) : 0;
    const bytes = Math.max(0, width * height * 4);
    this.textures.set(key, { texture, source, bytes });
    this.textureUploads += 1;
    this.textureBytes += bytes;
  }

  private deleteBatches(): void {
    for (const batch of this.batches) {
      this.gl.deleteBuffer(batch.vertexBuffer);
      this.gl.deleteBuffer(batch.slotBuffer);
      this.gl.deleteBuffer(batch.commandSlotBuffer);
      this.gl.deleteVertexArray(batch.vao);
    }
    this.batches = [];
  }
}

export function resourceIdentity(namespace: string, key: string): string {
  return `${namespace}\0${key}`;
}

function floatAttribute(gl: WebGL2RenderingContext, location: number, size: number, stride: number, offset: number): void {
  gl.enableVertexAttribArray(location);
  gl.vertexAttribPointer(location, size, gl.FLOAT, false, stride, offset);
}

function setStateTextureParameters(gl: WebGL2RenderingContext): void {
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
}

function createProgram(gl: WebGL2RenderingContext, vertexSource: string, fragmentSource: string): WebGLProgram {
  const compile = (type: number, source: string) => {
    const shader = gl.createShader(type);
    if (!shader) throw new Error("semantic shader allocation failed");
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(shader) ?? "semantic shader compile failed");
    return shader;
  };
  const vertex = compile(gl.VERTEX_SHADER, vertexSource);
  const fragment = compile(gl.FRAGMENT_SHADER, fragmentSource);
  const program = gl.createProgram();
  if (!program) throw new Error("semantic program allocation failed");
  gl.attachShader(program, vertex);
  gl.attachShader(program, fragment);
  gl.linkProgram(program);
  gl.deleteShader(vertex);
  gl.deleteShader(fragment);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program) ?? "semantic program link failed");
  return program;
}

const VERTEX_SHADER = `#version 300 es
precision highp float;
precision highp usampler2D;
layout(location=0) in vec2 a_position;
layout(location=1) in vec2 a_uv;
layout(location=2) in vec2 a_shapeUv;
layout(location=3) in vec4 a_fill;
layout(location=4) in vec4 a_stroke;
layout(location=5) in vec4 a_params;
layout(location=6) in uint a_layerSlot;
layout(location=7) in vec4 a_clip01;
layout(location=8) in vec4 a_clip23;
layout(location=9) in uint a_commandSlot;
uniform vec2 u_canvas;
uniform sampler2D u_state;
uniform highp usampler2D u_mask;
uniform float u_stateWidth;
uniform highp usampler2D u_commandMask;
uniform sampler2D u_commandState;
uniform float u_commandWidth;
out vec2 v_uv;
out vec2 v_shapeUv;
out vec4 v_fill;
out vec4 v_stroke;
out vec4 v_params;
out vec2 v_point;
out vec4 v_clip01;
out vec4 v_clip23;
flat out uint v_visible;
void main() {
  float stateU = (float(a_layerSlot) + 0.5) / u_stateWidth;
  vec2 dynamicOffset = texture(u_state, vec2(stateU, 0.5)).rg;
  float commandU = (float(a_commandSlot) + 0.5) / u_commandWidth;
  vec2 commandOffset = texture(u_commandState, vec2(commandU, 0.5)).rg;
  vec2 totalOffset = dynamicOffset + commandOffset;
  vec2 point = a_position + totalOffset;
  gl_Position = vec4(point.x / u_canvas.x * 2.0 - 1.0, 1.0 - point.y / u_canvas.y * 2.0, 0.0, 1.0);
  v_uv = a_uv;
  v_shapeUv = a_shapeUv;
  v_fill = a_fill;
  v_stroke = a_stroke;
  v_params = a_params;
  v_point = point;
  v_clip01 = a_clip01 + dynamicOffset.xyxy;
  v_clip23 = a_clip23 + dynamicOffset.xyxy;
  v_visible = texture(u_mask, vec2(stateU, 0.5)).r * texture(u_commandMask, vec2(commandU, 0.5)).r;
}`;

const SHAPE_FRAGMENT_SHADER = `#version 300 es
precision highp float;
in vec2 v_uv;
in vec2 v_shapeUv;
in vec4 v_fill;
in vec4 v_stroke;
in vec4 v_params;
in vec2 v_point;
in vec4 v_clip01;
in vec4 v_clip23;
flat in uint v_visible;
out vec4 outColor;
float shapeDistance() {
  if (v_params.x > 1.5) return length((v_shapeUv - 0.5) * 2.0) - 1.0;
  if (v_params.x > 0.5) {
    vec2 radius = max(v_params.yz, vec2(0.00001));
    vec2 q = abs(v_shapeUv - 0.5) - 0.5 + radius;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - min(radius.x, radius.y);
  }
  vec2 q = abs(v_shapeUv - 0.5) - 0.5;
  return max(q.x, q.y);
}
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
  if (v_visible == uint(0)) discard;
  if (!insideClip()) discard;
  float distance = shapeDistance();
  float aa = max(fwidth(distance), 0.0005);
  float fillCoverage = 1.0 - smoothstep(-aa, aa, distance);
  float strokeCoverage = v_params.w > 0.0 ? 1.0 - smoothstep(v_params.w - aa, v_params.w + aa, abs(distance)) : 0.0;
  vec4 color = mix(v_fill, v_stroke, strokeCoverage);
  color.a *= max(fillCoverage, strokeCoverage);
  color.rgb *= color.a;
  outColor = color;
}`;

const TEXTURE_FRAGMENT_SHADER = `#version 300 es
precision highp float;
in vec2 v_uv;
in vec2 v_shapeUv;
in vec4 v_fill;
in vec4 v_stroke;
in vec4 v_params;
in vec2 v_point;
in vec4 v_clip01;
in vec4 v_clip23;
flat in uint v_visible;
uniform sampler2D u_image;
uniform sampler2D u_alphaMask;
uniform int u_hasAlphaMask;
uniform int u_maskMode;
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
  if (v_visible == uint(0)) discard;
  if (!insideClip()) discard;
  if (v_params.x > 1.5 && length((v_shapeUv - 0.5) * 2.0) > 1.0) discard;
  if (v_params.x > 0.5 && v_params.x < 1.5) {
    vec2 radius = max(v_params.yz, vec2(0.00001));
    vec2 q = abs(v_shapeUv - 0.5) - 0.5 + radius;
    float distance = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - min(radius.x, radius.y);
    if (distance > 0.0) discard;
  }
  vec4 sampleColor = texture(u_image, v_uv);
  if (u_hasAlphaMask == 1) sampleColor *= texture(u_alphaMask, v_shapeUv).a;
  vec4 color;
  if (u_maskMode == 1) {
    float outlineSize = clamp(v_params.w, 0.0, 1.0);
    float faceThreshold = 0.5 + outlineSize * 0.2375;
    float outlineThreshold = min(1.0 - outlineSize * 0.95 * 0.75, 0.5);
    float sharp = 1.5 / 255.0;
    // Shape textures store the distance field in RGB and the valid sprite
    // domain in alpha. Transparent ASTC padding may legally contain R=1.0;
    // ignoring alpha turns that padding into a translucent face/outline veil.
    float faceCoverage = clamp((sampleColor.r - faceThreshold + sharp) / (2.0 * sharp), 0.0, 1.0) * sampleColor.a;
    float outerCoverage = clamp((sampleColor.r - outlineThreshold + sharp) / (2.0 * sharp), 0.0, 1.0) * sampleColor.a;
    float outlineCoverage = outerCoverage * (1.0 - faceCoverage);
    float faceAlpha = faceCoverage * floor(clamp(v_fill.a, 0.0, 1.0) * 255.0) / 255.0;
    float outlineAlpha = outlineCoverage * floor(clamp(v_stroke.a, 0.0, 1.0) * 255.0) / 255.0;
    color = vec4(
      v_fill.rgb * faceAlpha + v_stroke.rgb * outlineAlpha * (1.0 - faceAlpha),
      faceAlpha + outlineAlpha * (1.0 - faceAlpha)
    );
  } else {
    color = sampleColor * v_fill;
    color.rgb *= color.a;
  }
  outColor = color;
}`;
