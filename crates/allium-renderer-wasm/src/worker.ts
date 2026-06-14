/**
 * 渲染 Worker 入口。在 Worker 线程加载 wasm 并串行处理请求。
 *
 * 用法（主线程通过 `./worker-client` 间接使用，或手动）：
 * ```ts
 * const worker = new Worker(
 *   new URL("@allium/renderer-wasm/worker.js", import.meta.url),
 *   { type: "module" },
 * );
 * ```
 *
 * 协议见 `./protocol`。请求严格串行（单 AlliumRenderer 实例）。
 */

import { AlliumRenderer, ImageFormat } from "./renderer.js";
import type {
  EmscriptenModuleFactory,
} from "./emscripten.js";
import type {
  RequestMessage,
  ResponseMessage,
  RenderRequest,
  InitPayload,
} from "./protocol.js";

let renderer: AlliumRenderer | null = null;
let moduleUrl: string | null = null;
let wasmUrl: string | undefined;

async function ensureRenderer(): Promise<AlliumRenderer> {
  if (renderer) return renderer;
  if (!moduleUrl) throw new Error("Worker 未初始化（先发送 init 消息）");
  const factoryModule = (await import(/* @vite-ignore */ moduleUrl)) as {
    default: EmscriptenModuleFactory;
  };
  renderer = await AlliumRenderer.create(factoryModule.default, wasmUrl);
  return renderer;
}

function handleInit(payload: InitPayload): void {
  moduleUrl = payload.moduleUrl;
  wasmUrl = payload.wasmUrl;
  renderer = null; // 下次请求时按新 URL 重建
}

function applyInputs(r: AlliumRenderer, req: RenderRequest): void {
  for (const { family, bytes } of req.fonts) r.registerFont(family, bytes);
  for (const [name, json] of Object.entries(req.masterData)) {
    r.loadMasterData(name, json);
  }
  r.init();
  for (const { key, bytes } of req.assets) r.putAsset(key, bytes);
}

async function handle(msg: RequestMessage): Promise<{ result?: unknown; transfer: Transferable[] }> {
  switch (msg.kind) {
    case "init": {
      handleInit(msg.payload);
      return { transfer: [] };
    }
    case "render": {
      const r = await ensureRenderer();
      applyInputs(r, msg.payload);
      const out = r.render(
        msg.payload.cardJson,
        msg.payload.format ?? ImageFormat.Jpeg,
        msg.payload.profileJson,
      );
      return { result: out, transfer: [out.buffer] };
    }
    case "renderLayerCropped": {
      const r = await ensureRenderer();
      applyInputs(r, msg.payload);
      const out = r.renderLayerCropped(
        msg.payload.cardJson,
        msg.payload.quality ?? 80,
        msg.payload.profileJson,
      );
      return { result: out, transfer: [out.data.buffer] };
    }
    case "renderAllLayers": {
      const r = await ensureRenderer();
      applyInputs(r, msg.payload);
      const layers = r.renderAllLayers(
        msg.payload.cardJson,
        msg.payload.quality ?? 80,
        msg.payload.includeProperties ?? true,
        msg.payload.profileJson,
      );
      // 每层 data 是独立 Uint8Array，全部 transfer 回主线程，避免大缓冲克隆。
      const transfer: Transferable[] = layers
        .map((l) => l.data.buffer)
        .filter((b) => b.byteLength > 0);
      return { result: layers, transfer };
    }
    case "collectAssetKeys": {
      const r = await ensureRenderer();
      for (const [name, json] of Object.entries(msg.payload.masterData)) {
        r.loadMasterData(name, json);
      }
      r.init();
      const keys = r.collectAssetKeys(msg.payload.cardJson);
      return { result: keys, transfer: [] };
    }
  }
}

self.onmessage = async (ev: MessageEvent<RequestMessage>) => {
  const msg = ev.data;
  try {
    const { result, transfer } = await handle(msg);
    const res: ResponseMessage = { id: msg.id, ok: true, result };
    (self as unknown as Worker).postMessage(res, transfer);
  } catch (err) {
    const res: ResponseMessage = {
      id: msg.id,
      ok: false,
      error: err instanceof Error ? err.message : String(err),
    };
    (self as unknown as Worker).postMessage(res);
  }
};
