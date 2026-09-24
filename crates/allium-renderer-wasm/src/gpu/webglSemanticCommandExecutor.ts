import {
  SEMANTIC_COMMAND_SLOT_LOCATION,
  SEMANTIC_FLOATS_PER_VERTEX,
  SEMANTIC_LAYER_SLOT_LOCATION,
  SEMANTIC_VERTEX_ATTRIBUTES,
  semanticTextBatchKey,
  type SemanticBlendMode,
  type SemanticDrawBatch,
} from "./semanticCommandGeometry.js";
import type { SemanticCommandPlan, SemanticCommandStatePatch, SemanticLayerPatch } from "./semanticCommandPlanner.js";
import { COMMAND_CLIP_GLSL } from "./commandClipShader.js";
import { WebglSdfGlyphPipeline } from "./webglSdfGlyphPipeline.js";
import { WebglSdfAtlasTexture } from "./webglSdfAtlasTexture.js";
import { packPreviewTransformsForTexture } from "./previewTransformTextureLayout.js";
import type { SdfAtlas } from "../fontSdfAtlas.js";
import type { BrowserImageSource } from "./browserSemanticResources.js";
import type { UguiGlyphPage } from "../types/uguiText.js";

const PREVIEW_TRANSFORM_TEXTURE_UNIT = 5;
const ALPHA_MASK_TEXTURE_UNIT = 6;
const NORMAL_MAP_TEXTURE_UNIT = 7;

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
  private badgeProgram: WebGLProgram;
  private uguiGlyphProgram: WebGLProgram;
  private compositeVao: WebGLVertexArrayObject;
  private stateTexture: WebGLTexture;
  private maskTexture: WebGLTexture;
  private commandMaskTexture: WebGLTexture;
  private commandStateTexture: WebGLTexture;
  private previewTransformTexture: WebGLTexture;
  private batches: GpuBatch[] = [];
  private textures = new Map<string, { texture: WebGLTexture; source: BrowserImageSource; bytes: number }>();
  private uguiGlyphPages: WebGLTexture[] = [];
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
    // A lost context fails every compile; report the loss rather than a shader error.
    if (gl.isContextLost()) throw new Error("WebGL context is lost");
    const programs: WebGLProgram[] = [];
    const textures: WebGLTexture[] = [];
    let compositeVao: WebGLVertexArrayObject | null = null;
    let glyphPipeline: WebglSdfGlyphPipeline;
    try {
      programs.push(createProgram(gl, VERTEX_SHADER, SHAPE_FRAGMENT_SHADER));
      programs.push(createProgram(gl, VERTEX_SHADER, TEXTURE_FRAGMENT_SHADER));
      programs.push(createProgram(gl, COMPOSITE_VERTEX_SHADER, COMPOSITE_FRAGMENT_SHADER));
      programs.push(createProgram(gl, VERTEX_SHADER, BADGE_FRAGMENT_SHADER));
      programs.push(createProgram(gl, VERTEX_SHADER, UGUI_GLYPH_FRAGMENT_SHADER));
      compositeVao = gl.createVertexArray();
      for (let index = 0; index < 5; index += 1) {
        const texture = gl.createTexture();
        if (texture) textures.push(texture);
      }
      if (textures.length !== 5 || !compositeVao) throw new Error("semantic WebGL state texture creation failed");
      glyphPipeline = new WebglSdfGlyphPipeline(gl);
    } catch (error) {
      // A failed construction leaves nothing behind in the shared context.
      for (const program of programs) gl.deleteProgram(program);
      for (const texture of textures) gl.deleteTexture(texture);
      gl.deleteVertexArray(compositeVao);
      throw error;
    }
    [this.shapeProgram, this.textureProgram, this.compositeProgram, this.badgeProgram, this.uguiGlyphProgram] = programs;
    [this.stateTexture, this.maskTexture, this.commandMaskTexture, this.commandStateTexture, this.previewTransformTexture] = textures;
    this.compositeVao = compositeVao;
    this.glyphPipeline = glyphPipeline;
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
  }

  setTextGlyphBatch(commandId: string, vertices: Float32Array): void {
    this.glyphPipeline.upload(commandId, vertices);
  }

  /** Uploads the coverage pages the scene's `ugui_glyph` batches sample,
   * replacing any previous pages. */
  setUguiGlyphPages(pages: readonly UguiGlyphPage[]): { bytes: number } {
    const gl = this.gl;
    this.deleteUguiGlyphPages();
    let bytes = 0;
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    for (const page of pages) {
      const texture = gl.createTexture();
      if (!texture) throw new Error("uGUI glyph page allocation failed");
      this.uguiGlyphPages.push(texture);
      gl.bindTexture(gl.TEXTURE_2D, texture);
      // The fragment stage reads texels with texelFetch and filters them
      // itself, so the texture is never filtered.
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, page.width, page.height, 0, gl.RED, gl.UNSIGNED_BYTE, page.pixels);
      bytes += page.pixels.byteLength;
    }
    this.textureUploads += pages.length;
    this.textureBytes += bytes;
    return { bytes };
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
    // The loss event arrives asynchronously; until then a lost context answers
    // every query with null.
    if (gl.isContextLost()) throw new Error("WebGL context is lost");
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
        if (batch.source.kind === "ugui_glyph") {
          const page = this.uguiGlyphPages[batch.source.glyphPage ?? -1];
          if (!page) throw new Error(`uGUI glyph page ${String(batch.source.glyphPage)} is not uploaded`);
          gl.useProgram(this.uguiGlyphProgram);
          this.bindCommon(this.uguiGlyphProgram);
          gl.activeTexture(gl.TEXTURE2);
          gl.bindTexture(gl.TEXTURE_2D, page);
          gl.uniform1i(gl.getUniformLocation(this.uguiGlyphProgram, "u_glyphs"), 2);
          gl.bindVertexArray(batch.vao);
          gl.drawArrays(gl.TRIANGLES, 0, batch.vertices);
          drawCalls += 1;
          vertexBytes += batch.source.vertices.byteLength + batch.source.layerSlots.byteLength + batch.source.commandSlots.byteLength;
          continue;
        }
        const program = batch.source.kind === "shape"
          ? this.shapeProgram
          : batch.source.kind === "badge" ? this.badgeProgram : this.textureProgram;
        gl.useProgram(program);
        this.bindCommon(program);
        if (batch.source.kind !== "shape") {
          // The fragment stages read texels with texelFetch and filter them
          // themselves, so no sampler state depends on the batch.
          this.bindTexture(program, "u_image", 2, batch.source.resource, batch.source.kind);
          if (batch.source.kind === "badge") {
            this.bindTexture(program, "u_normalMap", NORMAL_MAP_TEXTURE_UNIT, batch.source.normalMapResource, batch.source.kind);
          } else {
            gl.uniform1i(gl.getUniformLocation(program, "u_maskMode"), batch.source.kind === "mask" ? 1 : 0);
          }
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
    this.deleteUguiGlyphPages();
    this.gl.deleteTexture(this.stateTexture);
    this.gl.deleteTexture(this.maskTexture);
    this.gl.deleteTexture(this.commandMaskTexture);
    this.gl.deleteTexture(this.commandStateTexture);
    this.gl.deleteTexture(this.previewTransformTexture);
    this.gl.deleteProgram(this.shapeProgram);
    this.gl.deleteProgram(this.textureProgram);
    this.gl.deleteProgram(this.compositeProgram);
    this.gl.deleteProgram(this.badgeProgram);
    this.gl.deleteProgram(this.uguiGlyphProgram);
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

  private bindTexture(
    program: WebGLProgram,
    uniform: string,
    unit: number,
    resource: { namespace: string; key: string } | null,
    kind: string,
  ): void {
    const gl = this.gl;
    if (!resource) throw new Error(`semantic ${kind} batch has no ${uniform} resource`);
    const key = resourceIdentity(resource.namespace, resource.key);
    const texture = this.textures.get(key)?.texture;
    if (!texture) throw new Error(`semantic GPU resource not loaded ${key}`);
    gl.activeTexture(gl.TEXTURE0 + unit);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.uniform1i(gl.getUniformLocation(program, uniform), unit);
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
    for (const { location, size, offset } of Object.values(SEMANTIC_VERTEX_ATTRIBUTES)) {
      floatAttribute(gl, location, size, stride, offset * 4);
    }
    gl.bindBuffer(gl.ARRAY_BUFFER, slotBuffer);
    gl.bufferData(gl.ARRAY_BUFFER, source.layerSlots, gl.STATIC_DRAW);
    gl.enableVertexAttribArray(SEMANTIC_LAYER_SLOT_LOCATION);
    gl.vertexAttribIPointer(SEMANTIC_LAYER_SLOT_LOCATION, 1, gl.UNSIGNED_INT, 4, 0);
    gl.bindBuffer(gl.ARRAY_BUFFER, commandSlotBuffer);
    gl.bufferData(gl.ARRAY_BUFFER, source.commandSlots, gl.STATIC_DRAW);
    gl.enableVertexAttribArray(SEMANTIC_COMMAND_SLOT_LOCATION);
    gl.vertexAttribIPointer(SEMANTIC_COMMAND_SLOT_LOCATION, 1, gl.UNSIGNED_INT, 4, 0);
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

  private deleteUguiGlyphPages(): void {
    for (const texture of this.uguiGlyphPages) this.gl.deleteTexture(texture);
    this.uguiGlyphPages = [];
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

/** Compiles and links one program; every shader and a failed program are
 * deleted before returning or throwing. */
function createProgram(gl: WebGL2RenderingContext, vertexSource: string, fragmentSource: string): WebGLProgram {
  const shaders: WebGLShader[] = [];
  const compile = (type: number, stage: string, source: string) => {
    const shader = gl.createShader(type);
    if (!shader) throw new Error(`semantic ${stage} shader allocation failed`);
    shaders.push(shader);
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
      throw new Error(gl.getShaderInfoLog(shader) || `semantic ${stage} shader compile failed`);
    }
    return shader;
  };
  try {
    const vertex = compile(gl.VERTEX_SHADER, "vertex", vertexSource);
    const fragment = compile(gl.FRAGMENT_SHADER, "fragment", fragmentSource);
    const program = gl.createProgram();
    if (!program) throw new Error("semantic program allocation failed");
    gl.attachShader(program, vertex);
    gl.attachShader(program, fragment);
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
      const log = gl.getProgramInfoLog(program);
      gl.deleteProgram(program);
      throw new Error(log || "semantic program link failed");
    }
    return program;
  } finally {
    for (const shader of shaders) gl.deleteShader(shader);
  }
}

const VERTEX_SHADER = `#version 300 es
precision highp float;
precision highp usampler2D;
layout(location=0) in vec4 a_inverse;
layout(location=1) in vec2 a_inverseOffset;
layout(location=2) in vec4 a_bounds;
layout(location=3) in vec2 a_corner;
layout(location=4) in vec4 a_uvRect;
layout(location=5) in vec4 a_fill;
layout(location=6) in vec4 a_stroke;
layout(location=7) in vec4 a_params;
layout(location=8) in vec4 a_gradient;
layout(location=9) in vec4 a_gradientEndColor;
layout(location=10) in vec4 a_clip01;
layout(location=11) in vec4 a_clip23;
layout(location=12) in uint a_layerSlot;
layout(location=13) in uint a_commandSlot;
uniform vec2 u_canvas;
uniform sampler2D u_state;
uniform highp usampler2D u_mask;
uniform float u_stateWidth;
uniform highp usampler2D u_commandMask;
uniform sampler2D u_commandState;
uniform float u_commandWidth;
uniform sampler2D u_previewTransform;
flat out vec4 v_inverse;
flat out vec2 v_inverseOffset;
flat out vec4 v_bounds;
flat out vec4 v_uvRect;
flat out vec4 v_fill;
flat out vec4 v_stroke;
flat out vec4 v_params;
flat out vec4 v_gradient;
flat out vec4 v_gradientEndColor;
flat out vec4 v_clip01;
flat out vec4 v_clip23;
flat out vec2 v_tangent;
flat out uint v_visible;
// Device pixels the quad is grown by on every side. The fragment stage
// decides what each pixel shows; the quad only has to reach every pixel with
// a sample inside the draw. No sample lies more than a quarter pixel from its
// pixel's centre along either axis, which leaves room for the rasteriser's
// snapping of the grown corners.
const float QUAD_OUTSET = 0.5;
void main() {
  float stateU = (float(a_layerSlot) + 0.5) / u_stateWidth;
  vec2 dynamicOffset = texture(u_state, vec2(stateU, 0.5)).rg;
  float commandU = (float(a_commandSlot) + 0.5) / u_commandWidth;
  vec2 commandOffset = texture(u_commandState, vec2(commandU, 0.5)).rg;
  vec2 totalOffset = dynamicOffset + commandOffset;
  vec4 preview0 = texelFetch(u_previewTransform, ivec2(int(a_layerSlot), 0), 0);
  vec4 preview1 = texelFetch(u_previewTransform, ivec2(int(a_layerSlot), 1), 0);
  // The canvas-to-local matrix after the dynamic offset and the preview: the
  // preview is undone, then the offset, then the draw's own mapping.
  float previewDeterminant = preview0.x * preview1.y - preview1.x * preview0.y;
  vec4 previewInverse = vec4(preview1.y, -preview1.x, -preview0.y, preview0.x) / previewDeterminant;
  vec2 previewInverseOffset = vec2(
    preview0.y * preview1.z - preview1.y * preview0.z,
    preview1.x * preview0.z - preview0.x * preview1.z
  ) / previewDeterminant;
  vec2 undone = previewInverseOffset - totalOffset;
  vec4 inverse = vec4(
    a_inverse.x * previewInverse.x + a_inverse.z * previewInverse.y,
    a_inverse.y * previewInverse.x + a_inverse.w * previewInverse.y,
    a_inverse.x * previewInverse.z + a_inverse.z * previewInverse.w,
    a_inverse.y * previewInverse.z + a_inverse.w * previewInverse.w
  );
  vec2 inverseOffset = vec2(
    a_inverse.x * undone.x + a_inverse.z * undone.y + a_inverseOffset.x,
    a_inverse.y * undone.x + a_inverse.w * undone.y + a_inverseOffset.y
  );
  // Local to canvas, for the quad's corners.
  float determinant = inverse.x * inverse.w - inverse.y * inverse.z;
  vec4 forward = vec4(inverse.w, -inverse.y, -inverse.z, inverse.x) / determinant;
  vec2 forwardOffset = vec2(
    inverse.z * inverseOffset.y - inverse.w * inverseOffset.x,
    inverse.y * inverseOffset.x - inverse.x * inverseOffset.y
  ) / determinant;
  // A canvas step of QUAD_OUTSET pixels moves a local coordinate by at most
  // QUAD_OUTSET times the length of that coordinate's row of the inverse, so
  // the bounds grown by that much reach every pixel within QUAD_OUTSET.
  vec2 outset = QUAD_OUTSET * vec2(length(inverse.xz), length(inverse.yw));
  vec2 local = a_bounds.xy + a_corner * a_bounds.zw + (a_corner * 2.0 - 1.0) * outset;
  vec2 point = vec2(
    forward.x * local.x + forward.z * local.y + forwardOffset.x,
    forward.y * local.x + forward.w * local.y + forwardOffset.y
  );
  gl_Position = vec4(point.x / u_canvas.x * 2.0 - 1.0, 1.0 - point.y / u_canvas.y * 2.0, 0.0, 1.0);
  v_inverse = inverse;
  v_inverseOffset = inverseOffset;
  v_bounds = a_bounds;
  v_uvRect = a_uvRect;
  v_fill = a_fill;
  v_stroke = a_stroke;
  v_params = a_params;
  v_gradient = a_gradient;
  v_gradientEndColor = a_gradientEndColor;
  vec2 clip0 = vec2(dot(preview0.xy, a_clip01.xy + dynamicOffset), dot(preview1.xy, a_clip01.xy + dynamicOffset)) + vec2(preview0.z, preview1.z);
  vec2 clip1 = vec2(dot(preview0.xy, a_clip01.zw + dynamicOffset), dot(preview1.xy, a_clip01.zw + dynamicOffset)) + vec2(preview0.z, preview1.z);
  vec2 clip2 = vec2(dot(preview0.xy, a_clip23.xy + dynamicOffset), dot(preview1.xy, a_clip23.xy + dynamicOffset)) + vec2(preview0.z, preview1.z);
  vec2 clip3 = vec2(dot(preview0.xy, a_clip23.zw + dynamicOffset), dot(preview1.xy, a_clip23.zw + dynamicOffset)) + vec2(preview0.z, preview1.z);
  v_clip01 = vec4(clip0, clip1);
  v_clip23 = vec4(clip2, clip3);
  // The local +x axis on the canvas, turned to the lighting frame where +y is up.
  vec2 axis = forward.xy;
  v_tangent = vec2(axis.x, -axis.y);
  v_visible = texture(u_mask, vec2(stateU, 0.5)).r * texture(u_commandMask, vec2(commandU, 0.5)).r;
}`;

// Declarations and helpers of every semantic fragment stage. Each stage maps
// the pixel under the fragment into the draw's local space itself; sampling
// positions are derived from the fragment position so they do not depend on
// the rasteriser's vertex precision.
const FRAGMENT_COMMON = `uniform vec2 u_canvas;
flat in vec4 v_inverse;
flat in vec2 v_inverseOffset;
flat in vec4 v_bounds;
flat in vec4 v_uvRect;
flat in vec4 v_fill;
flat in vec4 v_stroke;
flat in vec4 v_params;
flat in vec4 v_gradient;
flat in vec4 v_gradientEndColor;
flat in vec4 v_clip01;
flat in vec4 v_clip23;
flat in vec2 v_tangent;
flat in uint v_visible;
out vec4 outColor;
// Top-left corner of the canvas pixel under the fragment, +y down.
vec2 canvasPixel() {
  vec2 window = floor(gl_FragCoord.xy);
  return vec2(window.x, u_canvas.y - 1.0 - window.y);
}
// A canvas point in the draw's local space.
vec2 toLocal(vec2 point) {
  return vec2(
    v_inverse.x * point.x + (v_inverse.z * point.y + v_inverseOffset.x),
    v_inverse.y * point.x + (v_inverse.w * point.y + v_inverseOffset.y)
  );
}
${COMMAND_CLIP_GLSL}`;

// Image sampling shared by the image, mask and badge stages: the image clip
// and the texel under a sample, as the shared core decides them (a contract
// test pins both).
const IMAGE_SAMPLING = `bool imageClipContains(vec2 point) {
  if (v_params.x < 0.5) return true;
  float halfWidth = v_bounds.z * 0.5;
  float halfHeight = v_bounds.w * 0.5;
  if (halfWidth <= 0.0 || halfHeight <= 0.0) return false;
  if (v_params.x > 1.5) {
    float ellipseX = (point.x - (v_bounds.x + halfWidth)) / halfWidth;
    float ellipseY = (point.y - (v_bounds.y + halfHeight)) / halfHeight;
    return ellipseX * ellipseX + ellipseY * ellipseY <= 1.0;
  }
  float radiusX = min(abs(v_params.y), halfWidth);
  float radiusY = min(abs(v_params.z), halfHeight);
  if (radiusX == 0.0 || radiusY == 0.0) return true;
  float distanceX = abs(point.x - (v_bounds.x + halfWidth)) - (halfWidth - radiusX);
  float distanceY = abs(point.y - (v_bounds.y + halfHeight)) - (halfHeight - radiusY);
  if (distanceX <= 0.0 || distanceY <= 0.0) return true;
  float cornerX = distanceX / radiusX;
  float cornerY = distanceY / radiusY;
  return cornerX * cornerX + cornerY * cornerY <= 1.0;
}
ivec2 nearestTexel(vec2 coordinate, ivec2 size) {
  vec2 texel = floor(coordinate * vec2(size));
  return ivec2(clamp(texel, vec2(0.0), vec2(size - 1)));
}`;

// Shape coverage: the share of the pixel's samples inside the shape, each
// sample taking the stroke colour where the shape inset by the stroke width
// does not contain it and the fill elsewhere. A contract test pins it to the
// shared core rule.
const SHAPE_FRAGMENT_SHADER = `#version 300 es
precision highp float;
${FRAGMENT_COMMON}
// The straight fill colour at a local point: the gradient's colour at the
// point's projection onto its line, clamped to the line's ends. A shape
// without a gradient has an empty line and keeps its fill.
vec4 shapeFill(vec2 point) {
  float u = (point.x - v_bounds.x) / v_bounds.z;
  float v = (point.y - v_bounds.y) / v_bounds.w;
  float dx = v_gradient.z - v_gradient.x;
  float dy = v_gradient.w - v_gradient.y;
  float denominator = dx * dx + dy * dy;
  float t = denominator <= 1.1920929e-7 ? 0.0 : clamp(((u - v_gradient.x) * dx + (v - v_gradient.y) * dy) / denominator, 0.0, 1.0);
  return (v_gradientEndColor - v_fill) * t + v_fill;
}
bool shapeContains(vec2 point, float inset) {
  float left = v_bounds.x + inset;
  float top = v_bounds.y + inset;
  float right = v_bounds.x + v_bounds.z - inset;
  float bottom = v_bounds.y + v_bounds.w - inset;
  if (left >= right || top >= bottom || point.x < left || point.x >= right || point.y < top || point.y >= bottom) return false;
  if (v_params.x > 1.5) {
    float rx = (right - left) * 0.5;
    float ry = (bottom - top) * 0.5;
    float nx = (point.x - (left + right) * 0.5) / rx;
    float ny = (point.y - (top + bottom) * 0.5) / ry;
    return nx * nx + ny * ny <= 1.0;
  }
  if (v_params.x > 0.5) {
    float rx = min(max(v_params.y - inset, 0.0), (right - left) * 0.5);
    float ry = min(max(v_params.z - inset, 0.0), (bottom - top) * 0.5);
    if (rx == 0.0 || ry == 0.0) return true;
    float cx = clamp(point.x, left + rx, right - rx);
    float cy = clamp(point.y, top + ry, bottom - ry);
    float nx = (point.x - cx) / rx;
    float ny = (point.y - cy) / ry;
    return nx * nx + ny * ny <= 1.0;
  }
  return true;
}
void main() {
  if (v_visible == uint(0)) discard;
  vec2 pixel = canvasPixel();
  if (!insideClip(pixel + vec2(0.5))) discard;
  highp vec2 samples[4];
  samples[0] = vec2(0.25, 0.25);
  samples[1] = vec2(0.75, 0.25);
  samples[2] = vec2(0.25, 0.75);
  samples[3] = vec2(0.75, 0.75);
  vec4 accumulated = vec4(0.0);
  bool covered = false;
  for (int index = 0; index < 4; index += 1) {
    vec2 point = toLocal(pixel + samples[index]);
    if (!shapeContains(point, 0.0)) continue;
    covered = true;
    bool useStroke = v_params.w > 0.0 && !shapeContains(point, v_params.w);
    vec4 color = useStroke ? v_stroke : shapeFill(point);
    float alpha = clamp(color.a, 0.0, 1.0);
    accumulated += vec4(clamp(color.rgb, 0.0, 1.0) * alpha, alpha) * 0.25;
  }
  if (!covered) discard;
  outColor = accumulated;
}`;

// Images and shape masks: the texel under the pixel centre, drawn when the
// centre lies in the bounds and the image clip.
const TEXTURE_FRAGMENT_SHADER = `#version 300 es
precision highp float;
precision highp int;
${FRAGMENT_COMMON}
${IMAGE_SAMPLING}
uniform highp sampler2D u_image;
uniform highp sampler2D u_alphaMask;
uniform int u_hasAlphaMask;
uniform int u_maskMode;
void main() {
  if (v_visible == uint(0)) discard;
  vec2 centre = canvasPixel() + vec2(0.5);
  if (!insideClip(centre)) discard;
  vec2 local = toLocal(centre);
  float u = (local.x - v_bounds.x) / v_bounds.z;
  float v = (local.y - v_bounds.y) / v_bounds.w;
  if (u < 0.0 || u >= 1.0 || v < 0.0 || v >= 1.0) discard;
  if (!imageClipContains(local)) discard;
  vec2 imageUv = v_uvRect.xy + vec2(u, v) * v_uvRect.zw;
  vec4 sampleColor = texelFetch(u_image, nearestTexel(imageUv, textureSize(u_image, 0)), 0);
  // The mask spans the bounds and scales the premultiplied colour.
  float maskCoverage = 1.0;
  if (u_hasAlphaMask == 1) {
    maskCoverage = texelFetch(u_alphaMask, nearestTexel(vec2(u, v), textureSize(u_alphaMask, 0)), 0).a;
    if (maskCoverage == 0.0) discard;
  }
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
    // The face alpha reaches the game shader as an 8-bit vertex colour; the
    // outline colour is a float material colour.
    float faceAlpha = faceCoverage * roundEven(clamp(v_fill.a, 0.0, 1.0) * 255.0) / 255.0;
    float outlineAlpha = outlineCoverage * clamp(v_stroke.a, 0.0, 1.0);
    color = vec4(
      v_fill.rgb * faceAlpha + v_stroke.rgb * outlineAlpha * (1.0 - faceAlpha),
      faceAlpha + outlineAlpha * (1.0 - faceAlpha)
    );
  } else {
    vec4 tint = clamp(v_fill, 0.0, 1.0);
    color = sampleColor * tint;
    color.rgb *= color.a;
  }
  outColor = color * maskCoverage;
}`;

// The lit badge material; every constant matches the shared core material,
// which a contract test pins statement by statement. The albedo is the texel
// under the pixel centre and the normal map is filtered bilinearly at the
// same image position.
const BADGE_FRAGMENT_SHADER = `#version 300 es
precision highp float;
precision highp int;
${FRAGMENT_COMMON}
${IMAGE_SAMPLING}
uniform highp sampler2D u_image;
uniform highp sampler2D u_normalMap;
uniform highp sampler2D u_alphaMask;
uniform int u_hasAlphaMask;
vec4 normalTexel(vec2 coordinate) {
  ivec2 size = textureSize(u_normalMap, 0);
  vec2 last = vec2(size - 1);
  vec2 position = clamp(coordinate * vec2(size) - 0.5, vec2(0.0), last);
  vec2 low = floor(position);
  vec2 high = min(low + 1.0, last);
  vec2 weight = position - low;
  vec4 top = texelFetch(u_normalMap, ivec2(low.x, low.y), 0) * (1.0 - weight.x) + texelFetch(u_normalMap, ivec2(high.x, low.y), 0) * weight.x;
  vec4 bottom = texelFetch(u_normalMap, ivec2(low.x, high.y), 0) * (1.0 - weight.x) + texelFetch(u_normalMap, ivec2(high.x, high.y), 0) * weight.x;
  return top * (1.0 - weight.y) + bottom * weight.y;
}
// Specular term of one light: the unit direction towards it in xyz, its
// intensity in w.
float badgeSpecular(vec3 surface, vec4 light, float roughness2, float normalization) {
  vec3 halfDir = normalize(light.xyz + vec3(0.0, 0.0, -1.0));
  float lightHalf = clamp(dot(light.xyz, halfDir), 0.0, 1.0);
  float lightHalf2 = max(lightHalf * lightHalf, 0.1);
  float normalHalf = clamp(dot(surface, halfDir), 0.0, 1.0);
  float d = normalHalf * normalHalf * (roughness2 - 1.0) + 1.00001;
  float term = roughness2 / (d * d * lightHalf2 * normalization);
  return clamp(term - 6.1035156e-5, 0.0, 1000.0) * light.w;
}
void main() {
  if (v_visible == uint(0)) discard;
  vec2 centre = canvasPixel() + vec2(0.5);
  if (!insideClip(centre)) discard;
  vec2 local = toLocal(centre);
  float u = (local.x - v_bounds.x) / v_bounds.z;
  float v = (local.y - v_bounds.y) / v_bounds.w;
  if (u < 0.0 || u >= 1.0 || v < 0.0 || v >= 1.0) discard;
  if (!imageClipContains(local)) discard;
  vec2 imageUv = v_uvRect.xy + vec2(u, v) * v_uvRect.zw;
  // Both textures hold straight 8-bit values, lit in gamma space.
  vec4 albedo = texelFetch(u_image, nearestTexel(imageUv, textureSize(u_image, 0)), 0);
  vec4 packedNormal = normalTexel(imageUv);
  vec3 tangent = vec3(normalize(v_tangent), 0.0);
  vec3 normal = vec3(0.0, 0.0, -1.0);
  vec3 bitangent = cross(normal, tangent) * -1.0;
  float nx = packedNormal.r * packedNormal.a * 2.0 - 1.0;
  float ny = packedNormal.g * 2.0 - 1.0;
  float nz = max(sqrt(1.0 - min(nx * nx + ny * ny, 1.0)), 1e-16);
  // The perturbed normal is used as is, without renormalising.
  vec3 surface = nx * tangent + ny * bitangent + nz * normal;
  float roughness = max((1.0 - 0.6) * (1.0 - 0.6), 0.0078125);
  float roughness2 = roughness * roughness;
  float normalization = roughness * 4.0 + 30.0;
  float specular = 0.0;
  specular += badgeSpecular(surface, vec4(-0.25, 0.25881904, -0.9330127, 0.5), roughness2, normalization);
  specular += badgeSpecular(surface, vec4(0.4330127, 0.5, -0.75, 0.5), roughness2, normalization);
  specular += badgeSpecular(surface, vec4(0.5, -0.70710677, -0.5, 0.5), roughness2, normalization);
  float diffuse = 0.96 * (1.0 - 0.0);
  float alpha = (albedo.a >= 0.5 ? 1.0 : 0.0) * clamp(v_fill.a, 0.0, 1.0);
  vec3 color = clamp((albedo.rgb * diffuse + vec3(specular)) * v_fill.rgb * alpha, 0.0, 1.0);
  outColor = vec4(color, alpha);
  if (u_hasAlphaMask == 1) {
    float maskCoverage = texelFetch(u_alphaMask, nearestTexel(vec2(u, v), textureSize(u_alphaMask, 0)), 0).a;
    if (maskCoverage == 0.0) discard;
    outColor *= maskCoverage;
  }
}`;

// uGUI text glyphs: the coverage of the glyph cell at the pixel centre,
// interpolated between texel centres as the shared core samples a cell (a
// contract test pins it), times the vertex colour. The pixel is drawn when
// its centre lies in the cell; texels outside the bitmap are empty.
const UGUI_GLYPH_FRAGMENT_SHADER = `#version 300 es
precision highp float;
precision highp int;
${FRAGMENT_COMMON}
uniform highp sampler2D u_glyphs;
// Coverage of cell texel (column, row); the bitmap starts after the padding.
float glyphTexel(int column, int row) {
  ivec2 origin = ivec2(v_uvRect.xy);
  ivec2 size = ivec2(v_uvRect.zw);
  int padding = int(v_params.x);
  int x = column - padding;
  int y = row - padding;
  if (x < 0 || y < 0 || x >= size.x || y >= size.y) return 0.0;
  return texelFetch(u_glyphs, origin + ivec2(x, y), 0).r;
}
void main() {
  if (v_visible == uint(0)) discard;
  vec2 centre = canvasPixel() + vec2(0.5);
  if (!insideClip(centre)) discard;
  vec2 cell = toLocal(centre);
  if (cell.x < 0.0 || cell.x >= v_bounds.z || cell.y < 0.0 || cell.y >= v_bounds.w) discard;
  float x = cell.x - 0.5;
  float y = cell.y - 0.5;
  float left = floor(x);
  float top = floor(y);
  float fx = x - left;
  float fy = y - top;
  int column = int(left);
  int row = int(top);
  float upper = glyphTexel(column, row) + (glyphTexel(column + 1, row) - glyphTexel(column, row)) * fx;
  float lower = glyphTexel(column, row + 1) + (glyphTexel(column + 1, row + 1) - glyphTexel(column, row + 1)) * fx;
  float coverage = upper + (lower - upper) * fy;
  if (coverage <= 0.0) discard;
  vec4 color = clamp(v_fill, 0.0, 1.0);
  outColor = vec4(color.rgb * color.a, color.a) * coverage;
}`;

const COMPOSITE_VERTEX_SHADER = `#version 300 es
precision highp float;
void main() {
  highp vec2 positions[6];
  positions[0] = vec2(-1.0, -1.0);
  positions[1] = vec2(1.0, -1.0);
  positions[2] = vec2(1.0, 1.0);
  positions[3] = vec2(-1.0, -1.0);
  positions[4] = vec2(1.0, 1.0);
  positions[5] = vec2(-1.0, 1.0);
  gl_Position = vec4(positions[gl_VertexID], 0.0, 1.0);
}`;

// A group target has the canvas's size and orientation: each fragment copies
// the texel under it.
const COMPOSITE_FRAGMENT_SHADER = `#version 300 es
precision highp float;
uniform highp sampler2D u_image;
out vec4 outColor;
void main() {
  outColor = texelFetch(u_image, ivec2(gl_FragCoord.xy), 0);
}`;
