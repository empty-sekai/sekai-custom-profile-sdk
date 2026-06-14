# @allium/renderer-wasm

自定义名片渲染引擎的浏览器 WebAssembly 构建。skia CPU 光栅化 + FreeType
轮廓提取。输入名片 JSON，输出名片图片（JPEG / PNG / 透明 PNG）。

## 安装

```
npm install @allium/renderer-wasm
```

包内含构建好的 `allium_renderer_wasm.wasm` / `.js` 与 TypeScript wrapper。

## 资源由使用方注入

本包**不内嵌任何字体、masterdata 或素材**。运行前必须注入：

- **字体**：`registerFont(family, bytes)`。family 名需与 masterdata 中
  `customProfileTextFonts` 解析出的方正字体名一致（如 `FZLanTingHei-DB-GBK`）。
  缺字体的文本元素不会渲染。
- **masterdata**：`loadMasterData(name, json)` 逐表注入（约 18 张表，见
  下方 `REQUIRED_TABLES`），从你自己镜像的 `/masterdata/{region}/latest/`
  拷过来。
- **素材**：`collectAssetKeys(cardJson)` 给出名片所需 key，逐个
  `putAsset(key, bytes)` 注入。

## 用法

### 推荐：Web Worker（不阻塞 UI）

skia 光栅化是同步阻塞调用。浏览器中请用 Worker 客户端：

```ts
import { AlliumWorkerClient, ImageFormat } from "@allium/renderer-wasm/worker";

const client = await AlliumWorkerClient.spawn({
  workerUrl: new URL("@allium/renderer-wasm/worker.js", import.meta.url),
  moduleUrl: new URL("@allium/renderer-wasm/allium_renderer_wasm.js", import.meta.url).href,
});

const jpeg = await client.render({
  cardJson,                 // CustomProfileCard 或 UserCustomProfileCard[] 的 JSON
  profileJson,              // 可选：profile API 响应（注入 generals / 称号等级）
  format: ImageFormat.Jpeg, // 默认 JPEG
  masterData,               // Record<tableName, jsonText>
  fonts:  [{ family: "FZLanTingHei-DB-GBK", bytes: fontBytes }],
  assets: [{ key, bytes }], // 由 collectAssetKeys 收集
});
// jpeg: Uint8Array
```

> 注意：`render()` 默认转移 fonts/assets 的 ArrayBuffer 以避免大缓冲拷贝，
> 调用后这些 `Uint8Array` 会失效（detached）。需复用请先 `.slice()` 复制。

### 主线程直用（会阻塞）

```ts
import createAlliumRenderer from "@allium/renderer-wasm/allium_renderer_wasm.js";
import { AlliumRenderer, ImageFormat } from "@allium/renderer-wasm";

const r = await AlliumRenderer.create(createAlliumRenderer);
r.registerFont("FZLanTingHei-DB-GBK", fontBytes);
for (const [name, json] of Object.entries(masterData)) r.loadMasterData(name, json);
r.init();
for (const key of r.collectAssetKeys(cardJson)) r.putAsset(key, await fetchBytes(key));
const jpeg = r.render(cardJson, ImageFormat.Jpeg);
```

### 分层裁剪

`renderLayerCropped` 把所有可见元素绘到透明画布，裁剪到不透明像素的紧凑包围盒，
编码为 WebP，返回裁剪框在原画布坐标系的偏移。

```ts
// Worker 客户端
const layer = await client.renderLayerCropped({
  cardJson, profileJson, quality: 80, masterData, fonts, assets,
});
// layer: { data: Uint8Array /* WebP */, x, y, width, height }

// 主线程直用
const layer = r.renderLayerCropped(cardJson, 80, profileJson);
```

### 批量分层

`renderAllLayers` 把名片按 layer 升序逐元素渲染为裁剪 WebP，一次返回所有层
+ 元数据。`z` 字段为 layer 升序的 0-based 序号；不可见元素也出现在结果中，
`data` 为空、rect 全 0。

```ts
const layers = await client.renderAllLayers({
  cardJson, profileJson, quality: 80, includeProperties: true,
  masterData, fonts, assets,
});
// layers: Array<{
//   z, type, original_visible,
//   data: Uint8Array, x, y, width, height,
//   properties?: Record<string, unknown>,
// }>

// 主线程直用
const layers = r.renderAllLayers(cardJson, 80, true, profileJson);
```

`includeProperties=true` 时为每层填充 `properties`（字体名、颜色 hex、文本内容
等元素字段）；不需要时关掉省一遍 masterdata 查询。

## 从源码构建 wasm

需要 Docker（构建链固定在 `Dockerfile`：emsdk 4.0.10 / Rust 1.94）：

```
npm run build:wasm   # bash build.sh：docker build + 取出 dist/*.{js,wasm}
npm run build:ts     # tsc 编译 wrapper
```

## 许可

AGPL-3.0-only，附**浏览器范围 linking exception**（见 `LICENSE-EXCEPTION`）：
未修改的本制品在浏览器网页中运行时，与你的前端代码在浏览器内链接不使前端代码
受 AGPL 传染。修改引擎或在服务端/非浏览器环境运行，则适用完整 AGPL（含 §13
网络使用源码披露义务）。
