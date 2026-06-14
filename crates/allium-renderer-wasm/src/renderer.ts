/**
 * allium-renderer-wasm 核心封装。
 *
 * 直接在当前线程持有 emscripten 模块并调用 C ABI。skia CPU 光栅化是
 * 同步阻塞调用——主线程使用会卡 UI，故浏览器中建议用 `./worker` 的
 * Worker 客户端（本类在 Worker 内运行）。两者共享下方调用约定。
 *
 * 资源注入责任在使用方：本包不内嵌任何字体 / masterdata / 素材。
 * - 字体：`registerFont(family, bytes)`，缺字体的文本元素不渲染。
 * - masterdata：`loadMasterData(name, json)` 逐表注入后 `init()`。
 * - 素材：`putAsset(key, bytes)`（key 由 `collectAssetKeys` 给出）。
 */

import type { EmscriptenModule, EmscriptenModuleFactory } from "./emscripten.js";

/** 输出图片格式。 */
export enum ImageFormat {
  Jpeg = 0,
  Png = 1,
  PngTransparent = 2,
}

/** C ABI 错误（携带来自 `alr_last_error` 的引擎错误文本）。 */
export class AlliumRenderError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "AlliumRenderError";
  }
}

/** {@link AlliumRenderer.renderLayerCropped} 的输出：WebP 字节 + 画布坐标系裁剪框。 */
export interface CroppedLayerOutput {
  /** 裁剪后的 WebP 编码字节。 */
  data: Uint8Array;
  /** 裁剪框左上角 X（原画布坐标系）。 */
  x: number;
  /** 裁剪框左上角 Y（原画布坐标系）。 */
  y: number;
  /** 裁剪框宽度（像素）。完全透明时为 0。 */
  width: number;
  /** 裁剪框高度（像素）。完全透明时为 0。 */
  height: number;
}

/** {@link AlliumRenderer.renderAllLayers} 单层输出（WebP 字节 + 元数据）。 */
export interface LayerCrop {
  /** layer 升序的 0-based 序号。 */
  z: number;
  /** 元素类型 "text" / "card_member" / ...。 */
  type: string;
  /** 原始可见性（调用方可自行覆盖）。不可见层 `data` 为空。 */
  original_visible: boolean;
  /** 裁剪后的 WebP 字节；不可见层为空 Uint8Array。 */
  data: Uint8Array;
  /** 裁剪框（原画布坐标系）；不可见层全 0。 */
  x: number;
  y: number;
  width: number;
  height: number;
  /** 元素属性（字体名/颜色 hex/文本等），仅 `includeProperties=true` 时存在。 */
  properties?: Record<string, unknown>;
}

/** cwrap 出来的 C 函数签名集合。 */
interface Exports {
  alloc: (size: number) => number;
  free: (ptr: number, size: number) => void;
  lastError: (lenPtr: number) => number;
  loadMasterdata: (n: number, nl: number, j: number, jl: number) => number;
  registerFont: (f: number, fl: number, b: number, bl: number) => number;
  init: () => number;
  collectAssetKeys: (c: number, cl: number, outPtr: number, outLen: number) => number;
  putAsset: (k: number, kl: number, b: number, bl: number) => number;
  render: (
    c: number,
    cl: number,
    p: number,
    pl: number,
    fmt: number,
    outPtr: number,
    outLen: number,
  ) => number;
  renderLayerCropped: (
    c: number,
    cl: number,
    p: number,
    pl: number,
    quality: number,
    outPtr: number,
    outLen: number,
    outRect: number,
  ) => number;
  renderAllLayers: (
    c: number,
    cl: number,
    p: number,
    pl: number,
    quality: number,
    includeProps: number,
    outMetaPtr: number,
    outMetaLen: number,
    outBlobPtr: number,
    outBlobLen: number,
  ) => number;
}

const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder("utf-8");

export class AlliumRenderer {
  private constructor(
    private readonly mod: EmscriptenModule,
    private readonly ex: Exports,
  ) {}

  /**
   * 加载 wasm 模块并构造封装。
   *
   * @param factory 构建产物 `allium_renderer_wasm.js` 的默认导出。
   * @param wasmUrl `.wasm` 文件 URL（默认让 emscripten 在 .js 旁解析；
   *   打包/CDN 场景显式传入更可靠）。
   */
  static async create(
    factory: EmscriptenModuleFactory,
    wasmUrl?: string | URL,
  ): Promise<AlliumRenderer> {
    const mod = await factory(
      wasmUrl
        ? { locateFile: (path) => (path.endsWith(".wasm") ? String(wasmUrl) : path) }
        : undefined,
    );
    const ex: Exports = {
      alloc: mod.cwrap("alr_alloc", "number", ["number"]),
      free: mod.cwrap("alr_free", "void", ["number", "number"]),
      lastError: mod.cwrap("alr_last_error", "number", ["number"]),
      loadMasterdata: mod.cwrap("alr_load_masterdata", "number", [
        "number",
        "number",
        "number",
        "number",
      ]),
      registerFont: mod.cwrap("alr_register_font", "number", [
        "number",
        "number",
        "number",
        "number",
      ]),
      init: mod.cwrap("alr_init", "number", []),
      collectAssetKeys: mod.cwrap("alr_collect_asset_keys", "number", [
        "number",
        "number",
        "number",
        "number",
      ]),
      putAsset: mod.cwrap("alr_put_asset", "number", [
        "number",
        "number",
        "number",
        "number",
      ]),
      render: mod.cwrap("alr_render", "number", [
        "number",
        "number",
        "number",
        "number",
        "number",
        "number",
        "number",
      ]),
      renderLayerCropped: mod.cwrap("alr_render_layer_cropped", "number", [
        "number",
        "number",
        "number",
        "number",
        "number",
        "number",
        "number",
        "number",
      ]),
      renderAllLayers: mod.cwrap("alr_render_all_layers", "number", [
        "number",
        "number",
        "number",
        "number",
        "number",
        "number",
        "number",
        "number",
        "number",
        "number",
      ]),
    };
    return new AlliumRenderer(mod, ex);
  }

  /** 注入一张 masterdata 表（JSON 文本）。须在 {@link init} 前调用。 */
  loadMasterData(name: string, json: string): void {
    const n = this.pushStr(name);
    const j = this.pushStr(json);
    try {
      this.check(this.ex.loadMasterdata(n.ptr, n.len, j.ptr, j.len));
    } finally {
      this.popBuf(j);
      this.popBuf(n);
    }
  }

  /** 注册内存字体（family 名 + 字体文件字节）。本包不内嵌字体，必须显式注入。 */
  registerFont(family: string, bytes: Uint8Array): void {
    const f = this.pushStr(family);
    const b = this.pushBytes(bytes);
    try {
      this.check(this.ex.registerFont(f.ptr, f.len, b.ptr, b.len));
    } finally {
      this.popBuf(b);
      this.popBuf(f);
    }
  }

  /** 用已注入的表构建渲染器。重复调用等效热替换 masterdata。 */
  init(): void {
    this.check(this.ex.init());
  }

  /** 收集名片所需素材 key（{@link init} 之后调用）。 */
  collectAssetKeys(cardJson: string): string[] {
    const c = this.pushStr(cardJson);
    const outPtr = this.mod._malloc(4);
    const outLen = this.mod._malloc(4);
    try {
      this.check(this.ex.collectAssetKeys(c.ptr, c.len, outPtr, outLen));
      const json = this.takeOutput(outPtr, outLen);
      return JSON.parse(textDecoder.decode(json)) as string[];
    } finally {
      this.mod._free(outLen);
      this.mod._free(outPtr);
      this.popBuf(c);
    }
  }

  /** 注入素材（key + 编码图片字节）。 */
  putAsset(key: string, bytes: Uint8Array): void {
    const k = this.pushStr(key);
    const b = this.pushBytes(bytes);
    try {
      this.check(this.ex.putAsset(k.ptr, k.len, b.ptr, b.len));
    } finally {
      this.popBuf(b);
      this.popBuf(k);
    }
  }

  /**
   * 渲染名片，返回编码图片字节。
   *
   * @param cardJson `CustomProfileCard` 或 `UserCustomProfileCard[]`（取首张）JSON。
   * @param format 输出格式（默认 JPEG）。
   * @param profileJson 可选 profile API 响应 JSON（注入 generals / 称号等级）。
   */
  render(
    cardJson: string,
    format: ImageFormat = ImageFormat.Jpeg,
    profileJson?: string,
  ): Uint8Array {
    const c = this.pushStr(cardJson);
    const p = profileJson ? this.pushStr(profileJson) : { ptr: 0, len: 0 };
    const outPtr = this.mod._malloc(4);
    const outLen = this.mod._malloc(4);
    try {
      this.check(
        this.ex.render(c.ptr, c.len, p.ptr, p.len, format, outPtr, outLen),
      );
      return this.takeOutput(outPtr, outLen);
    } finally {
      this.mod._free(outLen);
      this.mod._free(outPtr);
      if (p.ptr) this.popBuf(p as Buf);
      this.popBuf(c);
    }
  }

  /**
   * 分层裁剪渲染：所有可见元素绘到透明画布，裁剪到不透明像素的紧凑包围盒，
   * 编码为 WebP，并返回裁剪框在原画布坐标系的偏移。
   *
   * @param cardJson `CustomProfileCard` 或 `UserCustomProfileCard[]`（取首张）JSON。
   * @param quality WebP 质量（0-100，默认 80）。
   * @param profileJson 可选 profile API 响应 JSON（注入 generals / 称号等级）。
   */
  renderLayerCropped(
    cardJson: string,
    quality = 80,
    profileJson?: string,
  ): CroppedLayerOutput {
    const c = this.pushStr(cardJson);
    const p = profileJson ? this.pushStr(profileJson) : { ptr: 0, len: 0 };
    const outPtr = this.mod._malloc(4);
    const outLen = this.mod._malloc(4);
    const outRect = this.mod._malloc(16); // 4 × u32: x, y, width, height
    try {
      this.check(
        this.ex.renderLayerCropped(
          c.ptr,
          c.len,
          p.ptr,
          p.len,
          quality,
          outPtr,
          outLen,
          outRect,
        ),
      );
      const data = this.takeOutput(outPtr, outLen);
      return {
        data,
        x: this.mod.getValue(outRect, "i32") >>> 0,
        y: this.mod.getValue(outRect + 4, "i32") >>> 0,
        width: this.mod.getValue(outRect + 8, "i32") >>> 0,
        height: this.mod.getValue(outRect + 12, "i32") >>> 0,
      };
    } finally {
      this.mod._free(outRect);
      this.mod._free(outLen);
      this.mod._free(outPtr);
      if (p.ptr) this.popBuf(p as Buf);
      this.popBuf(c);
    }
  }

  /**
   * 批量分层裁剪渲染：把名片按 layer 升序逐元素渲成裁剪 WebP，一次 FFI
   * 拿全部 N 层。
   *
   * @param cardJson `CustomProfileCard` 或 `UserCustomProfileCard[]`（取首张）JSON。
   * @param quality WebP 质量 0-100（默认 80）。
   * @param includeProperties 是否填充每层 `properties`（字体名/颜色 hex/文本等）；
   *   不需要时关掉省一遍 masterdata 查询。
   * @param profileJson 可选 profile API 响应 JSON（注入 generals / 称号等级）。
   *
   * 返回数组顺序 = layer 升序 = z 序号；不可见元素也在结果中（`data` 为空、
   * rect 全 0），便于完整重建图层列表。
   */
  renderAllLayers(
    cardJson: string,
    quality = 80,
    includeProperties = true,
    profileJson?: string,
  ): LayerCrop[] {
    const c = this.pushStr(cardJson);
    const p = profileJson ? this.pushStr(profileJson) : { ptr: 0, len: 0 };
    const outMetaPtr = this.mod._malloc(4);
    const outMetaLen = this.mod._malloc(4);
    const outBlobPtr = this.mod._malloc(4);
    const outBlobLen = this.mod._malloc(4);
    try {
      this.check(
        this.ex.renderAllLayers(
          c.ptr, c.len, p.ptr, p.len,
          quality,
          includeProperties ? 1 : 0,
          outMetaPtr, outMetaLen,
          outBlobPtr, outBlobLen,
        ),
      );
      const metaBytes = this.takeOutput(outMetaPtr, outMetaLen);
      const blobBytes = this.takeOutput(outBlobPtr, outBlobLen);
      const meta = JSON.parse(textDecoder.decode(metaBytes)) as Array<{
        z: number;
        type: string;
        original_visible: boolean;
        x: number; y: number; width: number; height: number;
        byte_offset: number; byte_length: number;
        properties?: Record<string, unknown>;
      }>;
      // 按 meta 切 blob。slice() 复制一份独立 Uint8Array（blobBytes 是 takeOutput
      // 复制出来的，引用切片虽然便宜但生命周期不直观；这里就显式复制）。
      return meta.map((m) => ({
        z: m.z,
        type: m.type,
        original_visible: m.original_visible,
        x: m.x, y: m.y, width: m.width, height: m.height,
        data: m.byte_length > 0
          ? blobBytes.slice(m.byte_offset, m.byte_offset + m.byte_length)
          : new Uint8Array(0),
        properties: m.properties,
      }));
    } finally {
      this.mod._free(outBlobLen);
      this.mod._free(outBlobPtr);
      this.mod._free(outMetaLen);
      this.mod._free(outMetaPtr);
      if (p.ptr) this.popBuf(p as Buf);
      this.popBuf(c);
    }
  }

  // ---- 内部 marshalling ----

  private pushStr(s: string): Buf {
    const bytes = textEncoder.encode(s);
    return this.pushBytes(bytes);
  }

  private pushBytes(bytes: Uint8Array): Buf {
    const len = bytes.length;
    // 长度 0 也分配 1 字节，避免 0 长度指针歧义。
    const ptr = this.ex.alloc(Math.max(len, 1));
    if (ptr === 0) throw new AlliumRenderError("alr_alloc 返回空指针（内存不足）");
    this.mod.HEAPU8.set(bytes, ptr);
    return { ptr, len, cap: Math.max(len, 1) };
  }

  private popBuf(buf: Buf): void {
    this.ex.free(buf.ptr, buf.cap);
  }

  /** 读取 `*out_ptr`/`*out_len` 指向的引擎输出缓冲并复制出来，随后 alr_free 释放。 */
  private takeOutput(outPtrPtr: number, outLenPtr: number): Uint8Array {
    const dataPtr = this.mod.getValue(outPtrPtr, "*");
    const dataLen = this.mod.getValue(outLenPtr, "*") >>> 0;
    // 复制出线性内存（HEAPU8 可能在后续调用因内存增长失效）。
    const copy = this.mod.HEAPU8.slice(dataPtr, dataPtr + dataLen);
    this.ex.free(dataPtr, dataLen);
    return copy;
  }

  private check(code: number): void {
    if (code === 0) return;
    throw new AlliumRenderError(this.readLastError());
  }

  private readLastError(): string {
    const lenPtr = this.mod._malloc(4);
    try {
      const ptr = this.ex.lastError(lenPtr);
      const len = this.mod.getValue(lenPtr, "*") >>> 0;
      if (ptr === 0 || len === 0) return "未知错误";
      return textDecoder.decode(this.mod.HEAPU8.slice(ptr, ptr + len));
    } finally {
      this.mod._free(lenPtr);
    }
  }
}

interface Buf {
  ptr: number;
  len: number;
  cap: number;
}
