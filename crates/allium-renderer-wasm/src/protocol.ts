/**
 * Worker 协议：主线程 ↔ 渲染 Worker 的消息类型。
 *
 * 渲染（skia CPU 光栅化）是同步阻塞，放进 Worker 避免卡主线程 UI。
 * 字节经 Transferable（ArrayBuffer）传递，避免结构化克隆复制大缓冲。
 */

import { ImageFormat } from "./renderer.js";

export { ImageFormat };
export type { CroppedLayerOutput, LayerCrop } from "./renderer.js";

/** 创建 Worker 时传入的初始化参数。 */
export interface InitPayload {
  /** wasm 工厂模块 URL（Worker 内 `import()` 加载）。 */
  moduleUrl: string;
  /** `.wasm` 文件 URL（可选，默认相对 moduleUrl 解析）。 */
  wasmUrl?: string;
}

/** 一次渲染请求的全部输入（一次性注入，渲染后 Worker 状态可复用）。 */
export interface RenderRequest {
  cardJson: string;
  profileJson?: string;
  format?: ImageFormat;
  /** 分层裁剪渲染的 WebP 质量（0-100，默认 80）。仅 renderLayerCropped / renderAllLayers 使用。 */
  quality?: number;
  /** renderAllLayers 是否填充每层 properties（默认 true）。 */
  includeProperties?: boolean;
  /** masterdata 表：name → JSON 文本。 */
  masterData: Record<string, string>;
  /** 字体：family → 字节。 */
  fonts: Array<{ family: string; bytes: Uint8Array }>;
  /** 素材：key → 字节。未提供的 key 渲染时按缺素材处理。 */
  assets: Array<{ key: string; bytes: Uint8Array }>;
}

export type RequestMessage =
  | { id: number; kind: "init"; payload: InitPayload }
  | { id: number; kind: "render"; payload: RenderRequest }
  | { id: number; kind: "renderLayerCropped"; payload: RenderRequest }
  | { id: number; kind: "renderAllLayers"; payload: RenderRequest }
  | { id: number; kind: "collectAssetKeys"; payload: { cardJson: string; masterData: Record<string, string> } };

export type ResponseMessage =
  | { id: number; ok: true; result?: unknown }
  | { id: number; ok: false; error: string };
