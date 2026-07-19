import { SEMANTIC_FLOATS_PER_VERTEX, semanticTextBatchKey, type SemanticBlendMode, type SemanticDrawBatch } from "./semanticCommandGeometry.js";
import type { SemanticCommandPlan, SemanticCommandStatePatch, SemanticLayerPatch } from "./semanticCommandPlanner.js";
import { WebglSdfGlyphPipeline } from "./webglSdfGlyphPipeline.js";
import { WebglSdfAtlasTexture } from "./webglSdfAtlasTexture.js";
import { packPreviewTransformsForTexture } from "./previewTransformTextureLayout.js";
import type { SdfAtlas } from "../fontSdfAtlas.js";
import type { BrowserImageSource } from "./browserSemanticResources.js";

const PREVIEW_TRANSFORM_TEXTURE_UNIT = 5;
const ALPHA_MASK_TEXTURE_UNIT = 6;

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

type IsolationTarget = {
  framebuffer: WebGLFramebuffer;
  texture: WebGLTexture;
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
  isolationBegins: number;
  isolationComposites: number;
  isolationTargetAllocations: number;
  isolationTextureBytes: number;
};

export class WebglSemanticCommandExecutor {
  private shapeProgram: WebGLProgram;
  private textureProgram: WebGLProgram;
  private compositeProgram: WebGLProgram;
  private compositeVao: WebGLVertexArrayObject;
  private stateTexture: WebGLTexture;
  private maskTexture: WebGLTexture;
  private commandMaskTexture: WebGLTexture;
  private commandStateTexture: WebGLTexture;
  private previewTransformTexture: WebGLTexture;
  private batches: GpuBatch[] = [];
  private textures = new Map<string, { texture: WebGLTexture; source: BrowserImageSource; bytes: number }>();
  private state = new Float32Array(2);
  private mask = new Uint8Array(1);
  private stateWidth = 1;
  private commandMask = new Uint8Array(1);
  private commandState = new Float32Array(2);
  private previewTransforms = new Float32Array([1, 0, 0, 0, 0, 1, 0, 0]);
  private commandWidth = 1;
  private plan: SemanticCommandPlan | null = null;
  private layerSlotById = new Map<string, number>();
  private readonly glyphPipeline: WebglSdfGlyphPipeline;
  private sdfAtlasTexture: WebglSdfAtlasTexture | null = null;
  private geometryBuilds = 0;
  private textureUploads = 0;
  private textureBytes = 0;
  private readonly isolationTargets: IsolationTarget[] = [];
  private isolationTargetAllocations = 0;

  constructor(private readonly gl: WebGL2RenderingContext, private readonly canvasWidth = CARD_W, private readonly canvasHeight = CARD_H) {
    this.shapeProgram = createProgram(gl, VERTEX_SHADER, SHAPE_FRAGMENT_SHADER);
    this.textureProgram = createProgram(gl, VERTEX_SHADER, TEXTURE_FRAGMENT_SHADER);
    this.compositeProgram = createProgram(gl, COMPOSITE_VERTEX_SHADER, COMPOSITE_FRAGMENT_SHADER);
    const compositeVao = gl.createVertexArray();
    const stateTexture = gl.createTexture();
    const maskTexture = gl.createTexture();
    const commandMaskTexture = gl.createTexture();
    const commandStateTexture = gl.createTexture();
    const previewTransformTexture = gl.createTexture();
    if (!stateTexture || !maskTexture || !commandMaskTexture || !commandStateTexture || !previewTransformTexture || !compositeVao) throw new Error("semantic WebGL state texture creation failed");
    this.stateTexture = stateTexture;
    this.maskTexture = maskTexture;
    this.commandMaskTexture = commandMaskTexture;
    this.commandStateTexture = commandStateTexture;
    this.previewTransformTexture = previewTransformTexture;
    this.compositeVao = compositeVao;
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

  setScene(plan: SemanticCommandPlan, batches: SemanticDrawBatch[], resources: Map<string, BrowserImageSource>): void {
    this.deleteBatches();
    this.glyphPipeline.clearBatches();
    this.plan = plan;
    const operations = plan.operations();
    this.layerSlotById = new Map(operations.map((operation) => [operation.layerId, operation.layerSlot] as const));
    this.stateWidth = Math.max(1, ...operations.map((operation) => operation.layerSlot + 1));
    this.state = new Float32Array(this.stateWidth * 2);
    this.mask = new Uint8Array(this.stateWidth);
    this.commandWidth = Math.max(1, ...operations.map((operation) => operation.commandSlot + 1));
    this.commandMask = new Uint8Array(this.commandWidth);
    this.commandState = new Float32Array(this.commandWidth * 2);
    this.previewTransforms = new Float32Array(this.stateWidth * 8);
    for (let slot = 0; slot < this.stateWidth; slot += 1) {
      this.previewTransforms.set([1, 0, 0, 0, 0, 1, 0, 0], slot * 8);
    }
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

  setLayerPreviewTransform(layerId: string, matrix: [number, number, number, number, number, number] | null): { previewUploadBytes: number } {
    if (!this.plan) throw new Error("semantic GPU scene is not initialized");
    const slot = this.layerSlotById.get(layerId);
    if (slot == null) throw new Error(`unknown preview layer ${layerId}`);
    const value = matrix ?? [1, 0, 0, 1, 0, 0];
    const offset = slot * 8;
    this.previewTransforms.set([value[0], value[2], value[4], 0, value[1], value[3], value[5], 0], offset);
    this.gl.bindTexture(this.gl.TEXTURE_2D, this.previewTransformTexture);
    this.gl.texSubImage2D(
      this.gl.TEXTURE_2D,
      0,
      slot,
      0,
      1,
      2,
      this.gl.RGBA,
      this.gl.FLOAT,
      this.previewTransforms.subarray(offset, offset + 8),
    );
    return { previewUploadBytes: 32 };
  }

  draw(): SemanticGpuMetrics {
    const gl = this.gl;
    const rootFramebuffer = gl.getParameter(gl.FRAMEBUFFER_BINDING) as WebGLFramebuffer | null;
    const rootViewport = gl.getParameter(gl.VIEWPORT) as Int32Array;
    gl.viewport(0, 0, this.canvasWidth, this.canvasHeight);
    gl.clearColor(1, 1, 1, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    let drawCalls = 0;
    let vertexBytes = 0;
    let isolationBegins = 0;
    let isolationComposites = 0;
    const isolationStack: IsolationTarget[] = [];
    try {
      for (const batch of this.batches) {
        if (batch.source.kind === "composite") {
          const operation = batch.source.compositeOperation;
          if (operation === "marker") continue;
          if (operation === "begin_isolation") {
            const target = this.isolationTarget(isolationStack.length);
            isolationStack.push(target);
            gl.bindFramebuffer(gl.FRAMEBUFFER, target.framebuffer);
            gl.viewport(0, 0, this.canvasWidth, this.canvasHeight);
            gl.clearColor(0, 0, 0, 0);
            gl.clear(gl.COLOR_BUFFER_BIT);
            isolationBegins += 1;
            continue;
          }
          if (operation === "end_isolation") {
            const target = isolationStack.pop();
            if (!target) throw new Error(`semantic isolation end has no matching begin: ${batch.source.commandIds[0] ?? "unknown"}`);
            gl.bindFramebuffer(gl.FRAMEBUFFER, isolationStack.at(-1)?.framebuffer ?? rootFramebuffer);
            gl.viewport(0, 0, this.canvasWidth, this.canvasHeight);
            this.drawIsolationTexture(target.texture);
            drawCalls += 1;
            isolationComposites += 1;
            continue;
          }
          throw new Error(`unsupported semantic composite operation ${String(operation)}`);
        }
        this.setBlendMode(batch.source.blendMode);
        if (batch.source.kind === "text") {
          const glyph = this.glyphPipeline.draw(
            semanticTextBatchKey(batch.source.commandIds),
            this.stateTexture,
            this.maskTexture,
            this.stateWidth,
            this.commandMaskTexture,
            this.commandStateTexture,
            this.commandWidth,
            this.previewTransformTexture,
          );
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
          gl.activeTexture(gl.TEXTURE0 + ALPHA_MASK_TEXTURE_UNIT);
          gl.bindTexture(gl.TEXTURE_2D, alphaMask);
          gl.uniform1i(gl.getUniformLocation(program, "u_alphaMask"), ALPHA_MASK_TEXTURE_UNIT);
          gl.uniform1i(gl.getUniformLocation(program, "u_hasAlphaMask"), alphaMask ? 1 : 0);
        }
        gl.bindVertexArray(batch.vao);
        gl.drawArrays(gl.TRIANGLES, 0, batch.vertices);
        drawCalls += 1;
        vertexBytes += batch.source.vertices.byteLength + batch.source.layerSlots.byteLength + batch.source.commandSlots.byteLength;
      }
      if (isolationStack.length !== 0) throw new Error(`semantic isolation has ${isolationStack.length} unclosed group(s)`);
    } finally {
      gl.bindFramebuffer(gl.FRAMEBUFFER, rootFramebuffer);
      gl.viewport(rootViewport[0], rootViewport[1], rootViewport[2], rootViewport[3]);
      this.setBlendMode("src_over");
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
      isolationBegins,
      isolationComposites,
      isolationTargetAllocations: this.isolationTargetAllocations,
      isolationTextureBytes: this.isolationTargets.length * this.canvasWidth * this.canvasHeight * 4,
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
    this.gl.deleteTexture(this.previewTransformTexture);
    this.gl.deleteProgram(this.shapeProgram);
    this.gl.deleteProgram(this.textureProgram);
    this.gl.deleteProgram(this.compositeProgram);
    this.gl.deleteVertexArray(this.compositeVao);
    for (const target of this.isolationTargets) {
      this.gl.deleteFramebuffer(target.framebuffer);
      this.gl.deleteTexture(target.texture);
    }
    this.isolationTargets.length = 0;
    this.glyphPipeline.destroy();
    this.sdfAtlasTexture?.destroy();
    this.layerSlotById.clear();
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
    gl.activeTexture(gl.TEXTURE0 + PREVIEW_TRANSFORM_TEXTURE_UNIT);
    gl.bindTexture(gl.TEXTURE_2D, this.previewTransformTexture);
    gl.uniform1i(gl.getUniformLocation(program, "u_previewTransform"), PREVIEW_TRANSFORM_TEXTURE_UNIT);
  }

  private setBlendMode(mode: SemanticBlendMode): void {
    const gl = this.gl;
    gl.blendEquation(gl.FUNC_ADD);
    if (mode === "src_over") gl.blendFuncSeparate(gl.ONE, gl.ONE_MINUS_SRC_ALPHA, gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    else if (mode === "src_in") gl.blendFuncSeparate(gl.DST_ALPHA, gl.ZERO, gl.DST_ALPHA, gl.ZERO);
    else if (mode === "dst_in") gl.blendFuncSeparate(gl.ZERO, gl.SRC_ALPHA, gl.ZERO, gl.SRC_ALPHA);
    else if (mode === "add") gl.blendFuncSeparate(gl.ONE, gl.ONE, gl.ONE, gl.ONE);
    else throw new Error(`semantic WebGL blend mode is not implemented: ${mode}`);
  }

  private isolationTarget(depth: number): IsolationTarget {
    const cached = this.isolationTargets[depth];
    if (cached) return cached;
    const gl = this.gl;
    const texture = gl.createTexture();
    const framebuffer = gl.createFramebuffer();
    if (!texture || !framebuffer) throw new Error(`semantic isolation allocation failed at depth ${depth}`);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, this.canvasWidth, this.canvasHeight, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
    if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) {
      gl.deleteFramebuffer(framebuffer);
      gl.deleteTexture(texture);
      throw new Error(`semantic isolation framebuffer is incomplete at depth ${depth}`);
    }
    const target = { framebuffer, texture };
    this.isolationTargets[depth] = target;
    this.isolationTargetAllocations += 1;
    return target;
  }

  private drawIsolationTexture(texture: WebGLTexture): void {
    const gl = this.gl;
    this.setBlendMode("src_over");
    gl.useProgram(this.compositeProgram);
    gl.activeTexture(gl.TEXTURE2);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.uniform1i(gl.getUniformLocation(this.compositeProgram, "u_image"), 2);
    gl.bindVertexArray(this.compositeVao);
    gl.drawArrays(gl.TRIANGLES, 0, 6);
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
    gl.bindTexture(gl.TEXTURE_2D, this.previewTransformTexture);
    setStateTextureParameters(gl);
    gl.texImage2D(
      gl.TEXTURE_2D,
      0,
      gl.RGBA32F,
      this.stateWidth,
      2,
      0,
      gl.RGBA,
      gl.FLOAT,
      packPreviewTransformsForTexture(this.previewTransforms, this.stateWidth),
    );
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
    floatAttribute(gl, 10, 2, stride, 26 * 4);
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

  private ensureTexture(key: string, source: BrowserImageSource): void {
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
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, source.source);
    const bytes = Math.max(0, source.width * source.height * 4);
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
layout(location=10) in vec2 a_shapeSize;
uniform vec2 u_canvas;
uniform sampler2D u_state;
uniform highp usampler2D u_mask;
uniform float u_stateWidth;
uniform highp usampler2D u_commandMask;
uniform sampler2D u_commandState;
uniform float u_commandWidth;
uniform sampler2D u_previewTransform;
out vec2 v_uv;
out vec2 v_shapeUv;
out vec4 v_fill;
out vec4 v_stroke;
out vec4 v_params;
out vec2 v_point;
out vec4 v_clip01;
out vec4 v_clip23;
out vec2 v_shapeSize;
flat out uint v_visible;
void main() {
  float stateU = (float(a_layerSlot) + 0.5) / u_stateWidth;
  vec2 dynamicOffset = texture(u_state, vec2(stateU, 0.5)).rg;
  float commandU = (float(a_commandSlot) + 0.5) / u_commandWidth;
  vec2 commandOffset = texture(u_commandState, vec2(commandU, 0.5)).rg;
  vec2 totalOffset = dynamicOffset + commandOffset;
  vec4 preview0 = texelFetch(u_previewTransform, ivec2(int(a_layerSlot), 0), 0);
  vec4 preview1 = texelFetch(u_previewTransform, ivec2(int(a_layerSlot), 1), 0);
  vec2 basePoint = a_position + totalOffset;
  vec2 point = vec2(
    dot(preview0.xy, basePoint) + preview0.z,
    dot(preview1.xy, basePoint) + preview1.z
  );
  gl_Position = vec4(point.x / u_canvas.x * 2.0 - 1.0, 1.0 - point.y / u_canvas.y * 2.0, 0.0, 1.0);
  v_uv = a_uv;
  v_shapeUv = a_shapeUv;
  v_fill = a_fill;
  v_stroke = a_stroke;
  v_params = a_params;
  v_point = point;
  vec2 clip0 = vec2(dot(preview0.xy, a_clip01.xy + dynamicOffset), dot(preview1.xy, a_clip01.xy + dynamicOffset)) + vec2(preview0.z, preview1.z);
  vec2 clip1 = vec2(dot(preview0.xy, a_clip01.zw + dynamicOffset), dot(preview1.xy, a_clip01.zw + dynamicOffset)) + vec2(preview0.z, preview1.z);
  vec2 clip2 = vec2(dot(preview0.xy, a_clip23.xy + dynamicOffset), dot(preview1.xy, a_clip23.xy + dynamicOffset)) + vec2(preview0.z, preview1.z);
  vec2 clip3 = vec2(dot(preview0.xy, a_clip23.zw + dynamicOffset), dot(preview1.xy, a_clip23.zw + dynamicOffset)) + vec2(preview0.z, preview1.z);
  v_clip01 = vec4(clip0, clip1);
  v_clip23 = vec4(clip2, clip3);
  v_shapeSize = a_shapeSize;
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
in vec2 v_shapeSize;
flat in uint v_visible;
out vec4 outColor;
float shapeDistance() {
  if (v_params.x > 1.5) {
    float localRadius = min(v_shapeSize.x, v_shapeSize.y) * 0.5;
    return (length((v_shapeUv - 0.5) * 2.0) - 1.0) * localRadius;
  }
  vec2 point = (v_shapeUv - 0.5) * v_shapeSize;
  vec2 halfSize = v_shapeSize * 0.5;
  if (v_params.x > 0.5) {
    vec2 radius = min(max(v_params.yz, vec2(0.00001)), halfSize);
    vec2 q = abs(point) - halfSize + radius;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - min(radius.x, radius.y);
  }
  vec2 q = abs(point) - halfSize;
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
  float strokeHalfWidth = v_params.w * 0.5;
  float strokeCoverage = v_params.w > 0.0 ? 1.0 - smoothstep(strokeHalfWidth - aa, strokeHalfWidth + aa, abs(distance)) : 0.0;
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
in vec2 v_shapeSize;
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
    vec2 point = (v_shapeUv - 0.5) * v_shapeSize;
    vec2 halfSize = v_shapeSize * 0.5;
    vec2 radius = min(max(v_params.yz, vec2(0.00001)), halfSize);
    vec2 q = abs(point) - halfSize + radius;
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

const COMPOSITE_VERTEX_SHADER = `#version 300 es
precision highp float;
out vec2 v_uv;
void main() {
  vec2 positions[6] = vec2[6](
    vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
    vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0)
  );
  vec2 position = positions[gl_VertexID];
  v_uv = position * 0.5 + 0.5;
  gl_Position = vec4(position, 0.0, 1.0);
}`;

const COMPOSITE_FRAGMENT_SHADER = `#version 300 es
precision highp float;
in vec2 v_uv;
uniform sampler2D u_image;
out vec4 outColor;
void main() {
  outColor = texture(u_image, v_uv);
}`;
