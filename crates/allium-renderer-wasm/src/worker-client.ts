/**
 * Worker 客户端：在主线程驱动渲染 Worker，返回 Promise。
 *
 * 这是浏览器中的推荐入口——skia 光栅化在 Worker 内同步执行，不阻塞 UI。
 *
 * ```ts
 * import { AlliumWorkerClient } from "@allium/renderer-wasm/worker";
 *
 * const client = await AlliumWorkerClient.spawn({
 *   workerUrl: new URL("@allium/renderer-wasm/worker.js", import.meta.url),
 *   moduleUrl: new URL("@allium/renderer-wasm/allium_renderer_wasm.js", import.meta.url).href,
 * });
 * const jpeg = await client.render({ cardJson, masterData, fonts, assets });
 * ```
 */

import { ImageFormat } from "./renderer.js";
import type {
  RequestMessage,
  ResponseMessage,
  RenderRequest,
  CroppedLayerOutput,
  LayerCrop,
} from "./protocol.js";

export { ImageFormat };
export type { RenderRequest, CroppedLayerOutput, LayerCrop } from "./protocol.js";

export interface SpawnOptions {
  /** Worker 脚本 URL（指向打包后的 `worker.js`）。 */
  workerUrl: string | URL;
  /** wasm 工厂模块 URL（Worker 内 import）。 */
  moduleUrl: string;
  /** `.wasm` 文件 URL（可选）。 */
  wasmUrl?: string;
}

export class AlliumWorkerClient {
  private nextId = 1;
  private readonly pending = new Map<
    number,
    { resolve: (v: unknown) => void; reject: (e: Error) => void }
  >();

  private constructor(private readonly worker: Worker) {
    this.worker.onmessage = (ev: MessageEvent<ResponseMessage>) => {
      const msg = ev.data;
      const entry = this.pending.get(msg.id);
      if (!entry) return;
      this.pending.delete(msg.id);
      if (msg.ok) entry.resolve(msg.result);
      else entry.reject(new Error(msg.error));
    };
    this.worker.onerror = (ev) => {
      const err = new Error(ev.message || "Worker 错误");
      for (const { reject } of this.pending.values()) reject(err);
      this.pending.clear();
    };
  }

  /** 启动 Worker 并完成初始化握手。 */
  static async spawn(opts: SpawnOptions): Promise<AlliumWorkerClient> {
    const worker = new Worker(opts.workerUrl, { type: "module" });
    const client = new AlliumWorkerClient(worker);
    await client.post({
      id: client.nextId++,
      kind: "init",
      payload: { moduleUrl: opts.moduleUrl, wasmUrl: opts.wasmUrl },
    });
    return client;
  }

  /** 渲染名片，返回编码图片字节。 */
  async render(req: RenderRequest): Promise<Uint8Array> {
    const result = await this.post(
      { id: this.nextId++, kind: "render", payload: req },
      this.collectTransfer(req),
    );
    return result as Uint8Array;
  }

  /**
   * 分层裁剪渲染，返回裁剪后的 WebP 字节及其在原画布的偏移。
   */
  async renderLayerCropped(req: RenderRequest): Promise<CroppedLayerOutput> {
    const result = await this.post(
      { id: this.nextId++, kind: "renderLayerCropped", payload: req },
      this.collectTransfer(req),
    );
    return result as CroppedLayerOutput;
  }

  /**
   * 批量分层裁剪渲染：一次返回所有元素的 WebP 字节 + 元数据。
   */
  async renderAllLayers(req: RenderRequest): Promise<LayerCrop[]> {
    const result = await this.post(
      { id: this.nextId++, kind: "renderAllLayers", payload: req },
      this.collectTransfer(req),
    );
    return result as LayerCrop[];
  }

  /** 收集名片所需素材 key。 */
  async collectAssetKeys(
    cardJson: string,
    masterData: Record<string, string>,
  ): Promise<string[]> {
    const result = await this.post({
      id: this.nextId++,
      kind: "collectAssetKeys",
      payload: { cardJson, masterData },
    });
    return result as string[];
  }

  /** 终止 Worker，拒绝所有在途请求。 */
  terminate(): void {
    this.worker.terminate();
    for (const { reject } of this.pending.values()) {
      reject(new Error("Worker 已终止"));
    }
    this.pending.clear();
  }

  private collectTransfer(req: RenderRequest): Transferable[] {
    // 转移字体/素材的 ArrayBuffer，避免结构化克隆复制大缓冲。
    // 注意：转移后调用方的 Uint8Array 会失效（detached）；如需复用请先复制。
    const transfer: Transferable[] = [];
    for (const f of req.fonts) transfer.push(f.bytes.buffer);
    for (const a of req.assets) transfer.push(a.bytes.buffer);
    return transfer;
  }

  private post(msg: RequestMessage, transfer: Transferable[] = []): Promise<unknown> {
    return new Promise((resolve, reject) => {
      this.pending.set(msg.id, { resolve, reject });
      this.worker.postMessage(msg, transfer);
    });
  }
}
