# @empty-sekai/sekai-custom-profile-sdk

[English](https://github.com/empty-sekai/sekai-custom-profile-sdk/blob/main/crates/sekai-profile-wasm/README.md) | 简体中文

面向 Project SEKAI 自定义名片的非官方浏览器 SDK。它根据 profile API 响应用 WebGL2 渲染自定义名片，编辑与游戏兼容的自定义名片文档，导出 PNG 快照，并输出图层、几何与交互区域，供宿主在其上构建自己的界面。

富文本与排版采用针对游戏所用 Unity TextMesh Pro 数据与行为的兼容模型，覆盖运行时已实现并测试的标签、排版、材质与动态行为。模型之外的行为以及后续游戏更新可能与游戏客户端不同，因此不保证完整复现客户端的渲染或像素。

Rust/WebAssembly 运行时运行在专用 worker 中，负责解析 profile、用 FreeType 度量排版文本，并把 glyph SDF 生成到 atlas。主线程调用宿主的 provider、解码图片并驱动 WebGL2。字体、profile 数据、masterdata 与图片都由宿主提供，包内不含任何此类资源。设计细节见[架构说明](https://github.com/empty-sekai/sekai-custom-profile-sdk/blob/main/docs/architecture.md)，原生渲染器与命令行工具见[仓库 README](https://github.com/empty-sekai/sekai-custom-profile-sdk#readme)。

## 目录

- [安装](#安装)
- [快速上手](#快速上手)
- [核心概念](#核心概念)
- [编辑文档](#编辑文档)
- [交互与状态](#交互与状态)
- [缓存与资源归属](#缓存与资源归属)
- [错误与告警](#错误与告警)
- [遥测与调试](#遥测与调试)
- [浏览器支持与构建](#浏览器支持与构建)
- [导出项](#导出项)
- [许可](#许可)

## 安装

```sh
npm install @empty-sekai/sekai-custom-profile-sdk
```

本包是带 TypeScript 声明的 ES module。`BrowserRenderer.create()` 与 `BrowserAuthoringClient.create()` 会从 `dist/worker.js` 启动 module worker，worker 再加载 `dist/sekai_profile_wasm.js` 与 `dist/sekai_profile_wasm.wasm`。这些 URL 相对本包自身的模块解析。若 bundler 或部署移动了这些文件，请把 `dist/` 作为一个整体目录提供（worker 会导入同目录的其他模块），并传入对应位置：

| 选项 | 默认值 | 作用 |
| --- | --- | --- |
| `workerUrl` | `dist/worker.js` | module worker 脚本。 |
| `moduleUrl` | `dist/sekai_profile_wasm.js` | worker 导入的 Emscripten 加载器。 |
| `wasmUrl` | `dist/sekai_profile_wasm.wasm` | WebAssembly 二进制。 |
| `workerFactory` | `new Worker(url, { type: "module" })` | 用 `workerUrl` 创建 worker，供需要自行构造 worker 的宿主使用。 |

## 快速上手

```ts
import {
  BrowserRenderer,
  type FontProvider,
  type ResourceProvider,
} from "@empty-sekai/sekai-custom-profile-sdk";

// loadApplicationFont、resolveResourceRequest 与 loadApplicationMasterData
// 代表宿主自己的字体、资源与 masterdata 来源。
const fontProvider: FontProvider = {
  async provide({ region, family }, { signal }) {
    const bytes = await loadApplicationFont({ region, family, signal });
    return bytes ? { bytes } : null;
  },
};

const resourceProvider: ResourceProvider = {
  async provide(descriptor, { signal }) {
    const request = await resolveResourceRequest(descriptor);
    if (!request) return null;
    const response = await fetch(request, { signal });
    if (!response.ok) return null;
    return { source: await response.blob() };
  },
};

const renderer = await BrowserRenderer.create({
  canvas: document.querySelector("canvas")!,
  region: "en",
  resourceProvider,
  fontProvider,
});

const masterData = await renderer.loadMasterData(
  "latest",
  ({ table, region, revision, optional }, { signal }) =>
    loadApplicationMasterData({ table, region, revision, optional, signal }),
);

const scene = await renderer.createProfileScene({
  masterData,
  documentKey: "profile-preview",
  profile, // 完整的 profile API 响应
  frameMode: "animate",
});

scene.draw();
const png = await scene.exportPng(); // 1830 × 812 的 PNG Blob
```

`profile` 是完整的 profile API 响应。渲染器按 `seq` 升序（即游戏中的页序）排列 `profile.userCustomProfileCards` 中的页，渲染第一页或 `pageIndex` 指定的页。响应的其余部分提供名片绘制所需的玩家数据，例如卡组、称号与歌曲成绩。

## 核心概念

### BrowserRenderer

一个 `BrowserRenderer` 持有一个 WebGL2 上下文、一个 worker 与各 provider 队列。每个 canvas 创建一个，并在该 canvas 上绘制的所有场景之间复用。

`BrowserRenderer.create()` 把 canvas 设为名片尺寸 1830 × 812；canvas 无法提供 WebGL2 上下文时以 `WEBGL2_UNAVAILABLE` 拒绝。

| 选项 | 默认值 | 说明 |
| --- | --- | --- |
| `canvas` | 必填 | `HTMLCanvasElement` 或 `OffscreenCanvas`。 |
| `region` | 必填 | 游戏区服：`cn`、`jp`、`tw`、`en` 或 `kr`。它是 masterdata session 的区服、每个字体请求的 `region`，也是场景的默认 locale。 |
| `resourceProvider` | 必填 | 图片来源，见 [ResourceProvider](#resourceprovider)。 |
| `resourceConcurrency` | `8` | `ResourceProvider.provide()` 的最大并发数。 |
| `fontProvider` | 无 | 字体来源，见 [FontProvider](#fontprovider)。 |
| `fontConcurrency` | `4` | `FontProvider.provide()` 的最大并发数。 |
| `localizationProvider` | 无 | 渲染器自有 UI 文本的来源，见 [LocalizationProvider](#localizationprovider)。 |
| `localizationConcurrency` | `8` | `LocalizationProvider.provide()` 的最大并发数。 |
| `prebuiltSdfAtlasProvider` | 无 | 预生成 glyph atlas 的来源，见[预生成 atlas 包](#预生成-atlas-包)。 |
| `telemetry` | `{ level: "summary" }` | 见[遥测与调试](#遥测与调试)。 |
| `workerUrl`、`moduleUrl`、`wasmUrl`、`workerFactory` | 包内文件 | 见[安装](#安装)。 |

并发数必须是正整数。

| 方法 | 说明 |
| --- | --- |
| `loadMasterData(revision, loadTable, options?)` | 创建 masterdata session，通过表加载函数填充并 seal。 |
| `createMasterData(revision)` | 创建空的 masterdata session，供逐表加载。 |
| `registerFont({ family, bytes })` | 以 family 名注册字体 bytes（`ArrayBuffer`）。 |
| `createProfileScene(options)` | 解析一页 profile，返回 `BrowserScene`。 |
| `stats()` | worker、provider、缓存与场景计数。 |
| `restoreContext()` | 重建丢失的 WebGL2 上下文并重新上传所有场景。 |
| `destroy()` | 终止 worker 并中止进行中的工作。 |

### Masterdata

masterdata session 持有一个区服、一个版本所需的游戏表。`loadMasterData()` 通过宿主的加载函数请求每张表，然后 seal 该 session。表的命名、位置、传输、鉴权与缓存由宿主决定：

```ts
const masterData = await renderer.loadMasterData(
  "catalog-2026-07",
  async ({ table, region, revision, optional }, { signal }) => {
    const response = await fetch(`/masterdata/${region}/${revision}/${table}.json`, { signal });
    if (optional && response.status === 404) return null;
    if (!response.ok) throw new Error(`${table}: HTTP ${response.status}`);
    return response.json();
  },
  { concurrency: 4 },
);
```

加载函数接收 `{ table, region, revision, optional }` 与一个 `AbortSignal`，返回解析后的表。`options.concurrency` 默认为 4；`options.signal` 用于中止加载。必需表加载失败时调用被拒绝，session 随之销毁。

`masterData.requiredTables` 列出每个 session 都需要的表。`masterData.optionalTables` 列出区服可能没有的表：只有部分区服提供的 `customProfileCharacterIconResources`、`customProfileMaterialResources` 与 `customProfileUserInterfaceIconResources`，以及只有御神签收藏品读取的 `omikujis`。加载这些表时 `optional` 为 `true`；返回 `null` 或 `undefined` 表示该区服没有这张表，可选表加载失败也会跳过。引用缺失表的元素不绘制，名片其余部分照常渲染。

需要逐表控制时，可直接填充 session：

```ts
const masterData = await renderer.createMasterData("catalog-2026-07");

for (const table of masterData.requiredTables) {
  await masterData.putTable(table, await applicationMasterData.load(table));
}
for (const table of masterData.optionalTables) {
  const value = await applicationMasterData.loadIfPresent(table);
  if (value != null) await masterData.putTable(table, value);
}

await masterData.seal();
```

session seal 之后才能创建场景。一个 session 可服务任意数量的场景；其场景用完后调用 `masterData.destroy()`。

### ResourceProvider

渲染器用语义描述符（descriptor）描述它需要的每张图片，宿主把描述符映射到 URL、对象存储、文件、鉴权请求或任意其他来源。

```ts
type ResourceDescriptor = {
  id: string;
  namespace: string;
  key: string;
  role: string;
  provenance: Record<string, unknown>;
  expectedSize?: { width: number; height: number };
};

interface ResourceProvider {
  provide(
    descriptor: ResourceDescriptor,
    context: { signal: AbortSignal },
  ): Promise<{ source: Blob | ArrayBuffer | Uint8Array | TexImageSource } | null>;

  cacheIdentity?(descriptor: ResourceDescriptor): string | null;
}
```

- `id` 是描述符的稳定标识。`namespace` 与 `key` 指明资源；`role` 与 `provenance` 说明是名片的哪一部分请求了它。这些是渲染器语义而非文件路径，由宿主解释。
- `source` 可以是编码后的图片 bytes（`Blob`、`ArrayBuffer`、`Uint8Array`），由渲染器用 `createImageBitmap()` 解码；也可以是已解码的 `TexImageSource`，例如 `ImageBitmap`。
- `cacheIdentity()` 决定解码图片 session 缓存的键，应包含会改变图片内容的目录版本、区服、用户范围或鉴权域。未提供时以描述符 `id` 为键。

一个 provider 可以组合内存、Cache Storage、IndexedDB、用户选择的文件、Service Worker、签名 URL 或鉴权接口，并完全掌握路径映射与失效策略：

```ts
const provider: ResourceProvider = {
  cacheIdentity(descriptor) {
    return `${assetRevision}:${descriptor.id}`;
  },

  async provide(descriptor, { signal }) {
    const stored = await assetDatabase.get(assetRevision, descriptor.id);
    if (stored) return { source: stored };

    const request = await applicationAssetRouter.resolve(descriptor);
    if (!request) return null;
    const response = await authenticatedFetch(request, { signal });
    if (!response.ok) return null;

    const blob = await response.blob();
    await assetDatabase.put(assetRevision, descriptor.id, blob);
    return { source: blob };
  },
};
```

调度与失败：

- 同一场景内的请求按描述符 `id` 去重；同一渲染器的并发场景共享进行中的请求。
- 每个等待的场景各自响应自己的取消；只有全部等待者都离开后，provider 收到的 signal 才会中止。
- 命中解码缓存时不调用 provider。
- 返回 `null`、抛出错误或解码失败会产生告警并使用透明占位，场景其余部分照常渲染，见[错误与告警](#错误与告警)。

有两类收藏品除自身图片外还会请求其他资源。罐徽章收藏品（`customProfileResourceCollectionType` 为 `can_badge`）还会请求法线贴图 `namespace: "static"`、`key: "ui/sekai_badge_normal"`。御神签收藏品按元素 `targetId` 指向的 `omikujis` 行绘制签，以两项 `assets` 资源请求签面图与运势图。

### FontProvider

字体 bytes 始终由宿主提供。`family` 是 masterdata 与文本元素使用的逻辑字体名，渲染器原样传给宿主。

```ts
type FontRequest = { region: string; family: string };

interface FontProvider {
  provide(
    request: FontRequest,
    context: { signal: AbortSignal },
  ): Promise<{ bytes: ArrayBuffer | Uint8Array } | null>;
}
```

每个场景由 worker 报告用到的 family，渲染器向 provider 请求其中尚未注册的部分，同时最多 `fontConcurrency` 个。返回的 bytes 会被复制、计算 SHA-256 并注册。返回 `null`、空文件或抛出错误都会拒绝场景创建，并中止其余进行中的请求。请提供 TrueType 或 OpenType 文件（包括 `.ttc` 字体集合）；内置的 FreeType 构建不含 zlib 与 Brotli，因此无法打开 WOFF 或 WOFF2 文件。

自行管理字体的宿主也可以在创建场景前直接注册：

```ts
const bytes = await file.arrayBuffer();

await renderer.registerFont({ family: "FOT-RodinNTLGPro-DB", bytes });
await renderer.registerFont({ family: "Application Alias", bytes });
```

同一文件可以注册为多个 family 名。某个 family 的首次注册在渲染器生命周期内固定其 source hash：用相同 bytes 再次注册不产生任何变化，用不同 bytes 注册则以 `FONT_IDENTITY_CONFLICT` 拒绝。切换字体版本请创建新的渲染器。glyph 标识包含区服、family、source hash 与字符，因此缓存的 glyph 不会混用字体版本。

御神签的签文是 uGUI 文本，直接按字体文件的 FreeType 覆盖率绘制，不使用 SDF atlas。签面字体在 2022 年签为 `FOT-Omikuji`，2023–2025 年签为 `FOT-UDMinchoPro-B`。签面字体缺少的字符取自第一个包含该字形的后备 face：先 `Roboto`，再按 masterdata 区服选取的 Noto Sans CJK face。`FontProvider` 会被请求这三个 family，返回完整文件，由渲染器打开下表所列的 face。这些 family 不会在预生成 atlas 中查找；缺少任何一个都会像场景用到的其他字体一样拒绝场景创建。

| Family | 文件 | Face |
| --- | --- | --- |
| `FOT-Omikuji` | `FOT-Omikuji.otf` | 0 |
| `FOT-UDMinchoPro-B` | `FOT-UDMinchoPro-B.otf` | 0 |
| `Roboto` | `Roboto-Regular.ttf` | 0 |
| `Noto Sans CJK SC`（区服 `cn`、`en` 及其他） | `NotoSansCJK-Regular.ttc` | 2 |
| `Noto Sans CJK TC`（区服 `tw`） | `NotoSansCJK-Regular.ttc` | 3 |
| `Noto Sans CJK JP`（区服 `jp`） | `NotoSansCJK-Regular.ttc` | 0 |
| `Noto Sans CJK KR`（区服 `kr`） | `NotoSansCJK-Regular.ttc` | 1 |

两个后备字体文件为 Android 14 自带的系统字体，其 SHA-256 见 [Fonts and assets](https://github.com/empty-sekai/sekai-custom-profile-sdk/blob/main/docs/fonts-and-assets.md)。

### LocalizationProvider

General 面板标题、标签页名、难度名等渲染器自有的 UI 文本按稳定的本地化键请求。玩家名称、签名等用户内容保持 profile 原文。

```ts
const renderer = await BrowserRenderer.create({
  canvas,
  region: "tw",
  resourceProvider,
  localizationProvider: {
    async provide({ region, locale, key }, { signal }) {
      return applicationMessages.resolve({ region, locale, key, signal });
    },
  },
  localizationConcurrency: 8,
});
```

worker 报告场景用到的键，渲染器对每个唯一键只解析一次，并把完整的 `key → 文本` 作为不可变快照交给 worker。provider 返回 `null` 或抛出错误时拒绝场景创建，不会改用其他区服的文本。未提供 `LocalizationProvider` 时，运行时使用该区服的内置文本表。

### 场景

`renderer.createProfileScene(options)` 把一页解析为 `BrowserScene`。

| 选项 | 默认值 | 说明 |
| --- | --- | --- |
| `masterData` | 必填 | 由同一渲染器创建且已 seal 的 session。 |
| `documentKey` | 必填 | 调用方选定的文档键。稳定图层 ID 由它和每个元素在名片中的位置派生，因此用同一个键重建同一页会得到相同的 ID。 |
| `profile` | 必填 | 包含 `userCustomProfileCards` 的完整 profile API 响应。 |
| `pageIndex` | `0` | 按 `seq` 升序的页序号，从 0 开始。 |
| `locale` | 渲染器的 `region` | 渲染器自有 UI 文本的 locale。 |
| `frameMode` | `"animate"` | `"animate"` 从 tick 0 开始运行动态程序；`"final"` 在创建时把它们推进到最终状态。 |
| `sdf.backend` | `"edt"` | glyph SDF 生成器：`"edt"`（CPU 欧氏距离变换）或用于调试与对照的 `"analytic"`。 |
| `sdf.supersample` | `2` | EDT 生成器的超采样倍数。 |
| `sdf.persistence` | `"origin"` | glyph 缓存范围：`"origin"`（IndexedDB）或 `"memory-only"`。 |
| `signal` | 无 | 在创建进行中取消创建。 |

profile 缺少 `userCustomProfileCards`、某页缺少 `customProfileCard` 或 `pageIndex` 超出页数时，以 `INVALID_PROFILE_INPUT` 拒绝。

场景创建依次执行：

1. 提供了 `LocalizationProvider` 时，worker 报告该页需要的本地化键，渲染器把它们解析为一份文本快照。
2. 提供了 `FontProvider` 或预生成 atlas provider 时，worker 报告该页用到的 font family。渲染器读取这些 family 的预生成 atlas manifest，并向 `FontProvider` 请求尚未注册的 family。
3. worker 结合 masterdata 解析该页，报告 glyph 需求、排版请求与资源描述符。预生成 atlas 覆盖场景全部 glyph 时直接使用它；否则 worker 用已注册字体生成 atlas，并复用已缓存的 glyph。
4. 渲染器向 `ResourceProvider` 请求图片。
5. worker 排版文本，编译图层树、语义命令、控件绑定、交互区域与初始动态状态；渲染器把 atlas 页、图片、几何与状态 buffer 上传到 WebGL2。

此后由宿主决定何时绘制、推进时间线、修改状态或读取 dump。

| 成员 | 说明 |
| --- | --- |
| `draw()` | 绘制当前状态并返回本帧的 GPU 计数。每次状态变化后调用。 |
| `exportPng()` | 先绘制，再把 canvas 编码为 1830 × 812 的 PNG `Blob`。在 document 中需要 Canvas 2D，其他环境需要 `OffscreenCanvas`。 |
| `advance(tick)` | 把动态时间线移到绝对 `tick`，每秒 60 tick。早于当前 tick 时从头重放动态程序。 |
| `animated` | 场景含需要 `advance()` 的动态程序时为 `true`。 |
| `dynamicProgramCount` | 场景中动态程序的数量。 |
| `dump()` | 图层表、几何与交互数据，见[交互与状态](#交互与状态)。 |
| `stats()` | 运行时快照，见[遥测与调试](#遥测与调试)。 |
| `destroy()` | 释放图片 lease、atlas lease、GPU 资源与 worker 内的场景。 |

取消与销毁：

```ts
const controller = new AbortController();
const pending = renderer.createProfileScene({
  masterData,
  documentKey: "preview",
  profile,
  signal: controller.signal,
});
controller.abort(); // 若创建仍在进行，`pending` 会被拒绝

const scene = await renderer.createProfileScene({ masterData, documentKey: "preview", profile });
await scene.destroy();
await masterData.destroy();
renderer.destroy();
```

signal 只取消进行中的创建；已创建的场景用 `destroy()` 释放。`renderer.destroy()` 终止 worker，并中止仍在进行的所有 provider 请求。

## 编辑文档

`BrowserAuthoringClient` 在自己的 worker 中编辑游戏格式的自定义名片文档。文档持有 `userCustomProfileCards` 各页；导入完整 profile 响应时只保留该数组，导出结果不含 worker 句柄、元素 ID、选中状态或其他编辑器状态。

```ts
import {
  BrowserAuthoringClient,
  type AuthoringCommand,
} from "@empty-sekai/sekai-custom-profile-sdk";

const authoring = await BrowserAuthoringClient.create();
const doc = profile
  ? await authoring.importProfile(profile)
  : await authoring.createBlank();

const command: AuthoringCommand = {
  kind: "set_transform",
  id: selectedElementId,
  position: [120, -40, 0],
};
const delta = await doc.apply(command);

await doc.undo();
await doc.redo();
const exported = await doc.export(); // { userCustomProfileCards: [...] }

await doc.destroy();
authoring.destroy();
```

`BrowserAuthoringClient.create()` 接受与渲染器相同的 `workerUrl`、`moduleUrl`、`wasmUrl` 与 `workerFactory` 选项。`createBlank()`、`importProfile(profile)` 与 `restoreCheckpoint(checkpoint)` 都返回 `RendererAuthoringDocument`：

| 成员 | 说明 |
| --- | --- |
| `apply(command)` | 把一条 `AuthoringCommand` 作为一条历史记录应用。 |
| `undo()`、`redo()` | 在历史中前后移动；无可撤销或重做时返回 `null`。 |
| `select(id)` | 选中元素；传 `null` 清除选中。 |
| `elements()` | 列出所有元素及其 `id`、`page`、`category`、`index` 与元素对象。 |
| `beginGesture(id)`、`previewGesture(command)`、`commitGesture()`、`cancelGesture()` | 对单个元素的连续编辑，见下文。 |
| `appendPage()`、`duplicatePage(page)`、`deletePage(page)`、`movePage(fromPage, page)` | 页操作。 |
| `export()` | 以 `GameProfileDocument` 返回文档。 |
| `checkpoint()` | 返回 `AuthoringCheckpoint`，`restoreCheckpoint()` 可连同历史与选中状态重新打开。 |
| `id`、`initialDocument`、`initialRevision` | session 句柄，以及打开时的文档与 revision。 |
| `destroy()` | 释放 worker 内的 session。 |

`AuthoringCommand` 的种类：

| `kind` | 字段 | 作用 |
| --- | --- | --- |
| `create` | `page`、`category`、`element` | 添加一个游戏格式的元素（含 `objectData`）。 |
| `duplicate` | `id` | 复制元素。 |
| `delete` | `id` | 删除元素。 |
| `set_transform` | `id`、`position?`、`scale?`、`rotation?` | 设置 `[x, y, z]` 位置与缩放，或 `[x, y, z, w]` 旋转。 |
| `set_lock` | `id`、`lock` | 锁定或解锁元素。 |
| `set_visible` | `id`、`visible` | 显示或隐藏元素。 |
| `set_parameters` | `id`、`values` | 设置 `objectData` 以外的元素字段。 |
| `change_layer` | `id`、`layer` | 在所在页的图层顺序中移动元素。 |

每个操作都返回 `AuthoringDelta`：新的 `revision`、元素变化 `changes`（`inserted`、`updated`、`removed`）、页变化 `pageChanges`（`inserted`、`removed`、`moved`）、`canUndo`、`canRedo` 与当前选中项。宿主把这些变化应用到自己的场景与界面；历史保存在 worker 中，宿主无需维护第二套命令历史。

手势用于拖动等连续编辑。`beginGesture(id)` 开始手势；`previewGesture()` 对该元素应用 `set_transform` 或 `set_parameters` 命令且不记入历史；`commitGesture()` 记入一条历史；`cancelGesture()` 还原元素。手势进行中其他文档命令会被拒绝，也不能创建 checkpoint。要在手势中移动已渲染的图层而不重建场景，可调用 `scene.setLayerPreviewTransform(layerId, matrix)`：`matrix` 是 canvas 像素坐标（原点在左上，y 向下）中的仿射矩阵 `[a, b, c, d, e, f]`，按 `x' = a·x + c·y + e`、`y' = b·x + d·y + f` 作用；传 `null` 清除。该调用只更新一个 GPU 矩阵并立即绘制。

编辑核心按游戏编辑器允许的范围校验：每页最多 150 个元素、最多 10 页、每页必备的 12 个元素数组、有限的变换数值、图层顺序以及 `objectData` 结构。历史保留最近 150 条。页序号是按 `seq` 升序的位置，与渲染器的 `pageIndex` 一致；页操作会把 `seq` 从 1 重新编号。要渲染编辑后的文档，把导出的 `userCustomProfileCards` 放入传给 `createProfileScene()` 的 `profile`。

## 交互与状态

场景状态变化在 worker 中执行，并更新紧凑的 GPU buffer。它们复用图层表、时间线、排版与 glyph atlas，并保持当前播放位置。

```ts
await scene.advance(tick);
await scene.setLayerVisible(layerId, false);
await scene.setLayerMasks(dump.revisions.layer_table, [
  { layerId, visible: true },
  { layerId: otherLayerId, visible: null },
]);
await scene.setTab(controlId, value);
await scene.setScrollOffset(controlId, offset);
await scene.scrollBy(controlId, delta);
scene.draw();
```

| 方法 | 说明 |
| --- | --- |
| `setLayerVisible(layerId, visible)` | 设置单个图层的 render mask。 |
| `setLayerMasks(layerTableRevision, overrides)` | 一次更新多个 mask；`visible: null` 清除覆盖。revision 必须等于 `dump.revisions.layer_table`，否则更新被拒绝。 |
| `setTab(controlId, value)` | 选中标签控件的某个标签。 |
| `setScrollOffset(controlId, offset)` | 设置滚动偏移；渲染器把它限制在内容范围内。 |
| `scrollBy(controlId, delta)` | 按 `delta` 滚动。 |
| `setLayerPreviewTransform(layerId, matrix)` | 应用仅作用于 GPU 的预览变换并绘制。 |

各异步方法返回 `{ delta, gpu }`：`delta` 是 worker 产生的 `CoreSceneDelta`（revision、tick、脏标记、图层与命令 patch），`gpu` 报告上传的字节数。它们不会绘制，之后请调用 `scene.draw()`。

公开图层对应游戏中编辑的元素。形状、glyph、mask 等绘制图元是归属于这些图层的命令。

### 场景 dump

`await scene.dump()` 返回名片设备坐标（`coordinate_space: "card-device-v1"`，1830 × 812）下的 `CoreSceneDump`，包含：

- `layer_table` 与 `layer_tree`：稳定图层 ID、slot、父节点与子树范围；
- `layers`：源内容、解析后的参数、编辑时的可见性、render mask 与动态状态；
- `interaction_regions`：命中区域，字段有 `role`、`bounds`、`quad`、`matrix`、`hit_geometry`、`clip`、`control_bindings`、`resolved_data`、`capabilities` 与 `render_mask`；
- `component_controls` 与 `command_states`；
- `revisions`、`tick` 与运行时计数；
- `numeric_text_regions`：去除 TMP 标记后在文本中识别出的连续数字，`resolved_data` 中给出纯文本及其范围。

区域的 `capabilities` 列出宿主可对其执行的操作，例如 `activate`、`scroll` 或 `drag`；`resolved_data` 携带解析后的目标，例如称号、卡面、角色、歌曲或剧情。控件绑定把区域关联到组件控件：`tab_option`（带标签的 `value`）、`scroll_viewport`、`scroll_content` 与 `scroll_thumb`。滚动组件分别暴露固定的 viewport、随偏移移动的内容与按比例移动的滑块。宿主在这些区域上处理滚轮与拖动输入，并调用 `setScrollOffset()` 或 `scrollBy()`；渲染器负责限制偏移、更新状态与命中几何。

产品交互由宿主实现：跳转到称号、角色、卡面、活动、歌曲或剧情；标签与滚动输入；复制数字；悬停卡片、选择与编辑；以及无障碍的 DOM 或 SVG 覆盖层。动画推进时保持覆盖层与图层列表的 DOM 稳定，只更新需要移动的部分。

## 缓存与资源归属

| 层级 | 生命周期 | 所有者 | 内容 |
| --- | --- | --- | --- |
| provider 存储 | 由宿主决定 | 宿主 | 编码后的图片及宿主的其他记录。 |
| 解码图片缓存 | 渲染器 | 主线程 | 由场景 lease 持有的解码图片。 |
| glyph 持久缓存 | 浏览器 origin | 渲染器 | IndexedDB 中经版本校验的 SDF glyph 记录。 |
| glyph session atlas | worker | WebAssembly 运行时 | atlas 页、放置位置、lease 与 revision。 |
| GPU 纹理与 buffer | WebGL 上下文 | 主线程 | atlas 页、图片、几何与状态 buffer。 |

解码图片缓存最多保留 64 MiB 已释放的图片，总量上限为 96 MiB。存活场景固定（pin）的图片占满 96 MiB 时，场景创建会被拒绝，而不会逐出这些图片。编码图片的持久化策略完全由 `ResourceProvider` 决定。

`sdf.persistence` 为默认的 `"origin"` 时，glyph 记录保存在 IndexedDB 数据库 `sekai-custom-profile-sdk-glyph-cache-v1` 中，由同一 origin 的所有渲染器共享。记录包含不透明的 glyph 标识、版本信息、度量与 R8 SDF 像素；超过 64 MiB 时淘汰最久未使用的记录。没有 IndexedDB 时缓存只在内存中；存储出错按缓存未命中处理，相应 glyph 会重新生成。`"memory-only"` 只在 session 内保留 glyph：

```ts
const scene = await renderer.createProfileScene({
  masterData,
  documentKey: "preview",
  profile,
  sdf: { persistence: "memory-only" },
});
```

WebGL 上下文丢失时，渲染器把所有场景标记为 `context-lost`，状态调用以 `WEBGL_CONTEXT_LOST` 拒绝。收到 `webglcontextrestored` 后，渲染器重建上下文并重新上传保留的 atlas、图片与 buffer 状态，复用已完成的文本排版与 SDF 生成结果。如需重试并以拒绝的形式拿到失败原因，可调用 `renderer.restoreContext()`。

### 预生成 atlas 包

默认情况下，场景按需生成 SDF glyph 并复用 glyph 缓存。预生成 atlas 包预先提供某个 family 的 glyph。包不会被自动下载：宿主要么通过 provider 提供，要么在用户明确操作后把它安装到 IndexedDB。

```ts
import {
  BrowserRenderer,
  createHttpPrebuiltSdfAtlasProvider,
  createOriginPrebuiltSdfAtlasPackage,
} from "@empty-sekai/sekai-custom-profile-sdk";

const source = createHttpPrebuiltSdfAtlasProvider("/font-atlases");
const atlasPackage = createOriginPrebuiltSdfAtlasPackage({
  namespace: "font-atlases-v1",
  source,
});

// 只在用户明确操作后调用。某个 family 的全部页写入后它才可见。
await atlasPackage.install(["FZLanTingHei-DB-GBK", "FZZhengHei-EB-GBK"], {
  concurrency: 4,
  requestPersistence: true,
  onProgress(progress) {
    updateDownloadProgress(progress.completedPages / progress.totalPages);
  },
});

const renderer = await BrowserRenderer.create({
  canvas,
  region: "cn",
  resourceProvider,
  fontProvider,
  prebuiltSdfAtlasProvider: atlasPackage.provider,
});
```

- `createHttpPrebuiltSdfAtlasProvider(baseUrl)` 读取 `<baseUrl>/<family>/manifest.json` 及其旁边的页文件。manifest 返回 404 表示该 family 没有包；其他 HTTP 错误会拒绝场景创建。
- `createOriginPrebuiltSdfAtlasPackage({ namespace, source })` 把 `source` 中的包安装到 IndexedDB 数据库 `sekai-custom-profile-sdk-prebuilt-atlas-v1` 的 `namespace` 下。它的 `provider` 只返回已完整安装的 family；未安装、未装完或存储不可用时返回 `null`，相应场景改为按需生成 glyph。
- `install(families, options)` 跳过已安装的 family，检查浏览器报告的配额，校验每页的 SHA-256，并在安装失败时删除本次的 family。选项：`concurrency`（默认 4，最多 8）、`onProgress`、`requestPersistence`（默认 `false`；会调用 `navigator.storage.persist()`）与 `signal`。结果为 `PrebuiltSdfAtlasPackageStatus`。
- `status(families)`、`remove(families?)`（不带参数时删除该 namespace 的全部 family）与 `close()` 构成其余的包 API。

不需要浏览器持久化时，可把 `createHttpPrebuiltSdfAtlasProvider()` 直接作为 `prebuiltSdfAtlasProvider`，由本地文件、Service Worker 或 HTTP 缓存管理生命周期。包只保存 glyph 页与 manifest，不含 profile、文本、用户 ID、排版或场景 dump。包不替代字体文件：`FontProvider` 仍会被请求场景中尚未注册的 family。每个场景都会重新向 provider 查询 manifest，因此安装或删除从下一个场景起生效。

包即 `sekai-profile` crate 的 `build-sdf-atlas` 工具的输出，原样提供即可：

```sh
SEKAI_PROFILE_FONT_DIR=/path/to/fonts cargo run -p sekai-profile --release \
  --bin build-sdf-atlas -- --font-family FZLanTingHei-DB-GBK \
  --output font-atlases/FZLanTingHei-DB-GBK
```

目录名与 manifest 的 `font_family` 必须等于场景请求的 family 名。每页的 `file_sha256` 针对解压后的页内容，因此当服务端以 `Content-Encoding: br` 返回时，页文件可以预压缩存为 `page-NNN.r8swz.br`（manifest 中的文件名同步改名）。页尺寸最大 2048 × 2048。同一场景用到的各 family 必须使用相同的 `--point-size`、`--spread` 与 `--page-size`，否则该场景改为按需生成 glyph。工具如何查找字体文件见 [Fonts and assets](https://github.com/empty-sekai/sekai-custom-profile-sdk/blob/main/docs/fonts-and-assets.md)。

## 错误与告警

会导致场景错误的失败一律显式拒绝。`BrowserRendererError` 带有字符串 `code`：

| Code | 原因 |
| --- | --- |
| `WEBGL2_UNAVAILABLE` | canvas 无法提供 WebGL2 上下文。 |
| `WEBGL_CONTEXT_LOST` | 在 WebGL 上下文丢失期间调用。 |
| `WEBGL2_RESTORE_FAILED` | `restoreContext()` 无法创建新上下文。 |
| `RENDERER_DESTROYED`、`SCENE_DESTROYED` | 在 `destroy()` 之后调用。 |
| `INVALID_PROFILE_INPUT` | profile 中没有可用的页，或 `pageIndex` 越界。 |
| `MASTERDATA_LOADER_REQUIRED`、`INVALID_MASTERDATA_CONCURRENCY` | `loadMasterData()` 参数无效。 |
| `INVALID_FONT_FAMILY`、`INVALID_FONT_BYTES` | family 名或字体 bytes 为空。 |
| `FONT_IDENTITY_CONFLICT` | 同一 family 以不同 bytes 再次注册。 |
| `FONT_NOT_REGISTERED` | 场景需要的 family 尚未注册。 |
| `PNG_EXPORT_UNAVAILABLE`、`PNG_EXPORT_FAILED` | 没有 Canvas 2D 或 `OffscreenCanvas`，或 PNG 编码没有返回数据。 |

worker 内产生的错误以 `RendererWorkerError` 拒绝，其 `code` 指明失败类型。`WASM_CALL_FAILED` 表示运行时拒绝了输入，例如 masterdata session 未 seal、masterdata 无效或编辑命令超出限制，具体原因见 message。`WORKER_CRASHED` 与 `WORKER_TERMINATED` 会拒绝所有待处理的调用。atlas 包的 `install()` 以 `PrebuiltSdfAtlasStorageError` 拒绝，code 为 `PREBUILT_ATLAS_STORAGE_UNAVAILABLE`、`PREBUILT_ATLAS_INVALID_MANIFEST`、`PREBUILT_ATLAS_QUOTA` 或 `PREBUILT_ATLAS_PAGE_HASH`。

各 provider 的结果：

| Provider | `null`、错误或数据无效 | 中止 |
| --- | --- | --- |
| `ResourceProvider` | 告警并使用透明占位，场景照常渲染。 | 拒绝场景创建。 |
| `FontProvider` | 拒绝场景创建。 | 拒绝场景创建。 |
| `LocalizationProvider` | 拒绝场景创建。 | 拒绝场景创建。 |
| masterdata 加载函数，必需表 | 拒绝 `loadMasterData()`。 | 拒绝 `loadMasterData()`。 |
| masterdata 加载函数，可选表 | 跳过该表，引用它的元素不绘制。 | 拒绝 `loadMasterData()`。 |

资源告警输出到控制台，带前缀 `[sekai-custom-profile-sdk]`、描述符的 `namespace`、`key`、`role` 与原因，同时计入 `renderer.stats().resources.provider.failures`。

## 遥测与调试

```ts
const renderer = await BrowserRenderer.create({
  canvas,
  region: "en",
  resourceProvider,
  telemetry: { level: "trace", maxSamples: 240 },
});

const sceneStats = scene.stats();
const rendererStats = await renderer.stats();
```

`telemetry.level` 默认为 `"summary"`：每个场景保留最近 `maxSamples` 帧（1–240，默认 240）帧耗时的 p50、p95 与最大值。`"trace"` 还保留原始帧样本；`"off"` 不记录帧样本，运行时计数仍然可用。

`scene.stats()` 返回 `schemaVersion: 1` 的快照：场景状态（`ready`、`context-lost` 或 `destroyed`）、帧数、初始化上传量、atlas 页数、glyph 数量与生成耗时、glyph 缓存命中与未命中、最近一帧的 GPU 计数、状态上传字节数、上下文丢失与恢复次数，以及遥测样本。不可用或 disjoint 的 GPU 计时为 `null`，不会记为零。

`renderer.stats()` 返回：

- `worker`：worker 请求数、失败数、WebAssembly 耗时、桥接字节数与打开的 session；
- `resources`：解码缓存的条目、字节、固定数、命中、加载与逐出，以及 `provider` 的调用数、峰值并发、已知编码字节、失败、取消与解析耗时；
- `fonts`：已注册的 family，以及 `FontProvider` 的请求数、字节、失败、峰值并发与耗时；
- `localizations`：`LocalizationProvider` 的请求数、解析数、失败、峰值并发与耗时；
- `scenes`：活动场景数与等待上下文恢复的场景数。

遥测只记录计数、耗时、容量与恢复状态，不包含源文本、稳定 ID、资源 URL、字体名或玩家数据。

## 浏览器支持与构建

运行时需要：

- WebGL2；
- ES modules 与 module Web Worker；
- 支持异常处理的 WebAssembly（运行时以 `-fwasm-exceptions` 编译）；
- 安全上下文，因为字体与 atlas 页的哈希使用 `crypto.subtle`；
- 用于解码编码图片的 `createImageBitmap()`；
- 可选的 IndexedDB，用于 glyph 缓存与预生成 atlas 包。

运行时内的 FreeType 构建只包含 TrueType、CFF、SFNT、PSaux、PSNames 与 smooth 光栅化模块。CPU EDT 是默认的 glyph SDF 生成器；analytic 生成器用于调试与对照。

从仓库检出构建本包：

```sh
cd crates/sekai-profile-wasm
npm ci
npm run build        # WebAssembly 运行时（build.sh）与 TypeScript
npm run typecheck
npm run test:gates
npm run demo:serve   # 工作台地址 http://127.0.0.1:8088/demo/
```

`npm run build:wasm` 执行 `build.sh`：它在 `Dockerfile` 定义的 Docker 镜像中（含 Emscripten 与固定版本的 Rust 工具链）构建运行时；没有 Docker 且 `PATH` 中有 `emcc` 与 `cargo` 时直接构建。全部脚本与 CI 检查见 [Development](https://github.com/empty-sekai/sekai-custom-profile-sdk/blob/main/docs/development.md)，工作台见 [demo README](https://github.com/empty-sekai/sekai-custom-profile-sdk/blob/main/crates/sekai-profile-wasm/demo/README.md)。

## 导出项

| 导出 | 类别 | 用途 |
| --- | --- | --- |
| `BrowserRenderer`、`BrowserScene` | class | 渲染与场景状态。 |
| `BrowserRendererError` | class | 带 `code` 的错误。 |
| `RendererWorkerError` | class | worker 抛出的带 `code` 的错误。 |
| `BrowserAuthoringClient`、`RendererAuthoringDocument` | class | 文档编辑。 |
| `AUTHORING_CHECKPOINT_SCHEMA` | constant | `AuthoringCheckpoint` 的 schema 标记。 |
| `createHttpPrebuiltSdfAtlasProvider`、`createOriginPrebuiltSdfAtlasPackage` | function | 预生成 atlas 来源。 |
| `isValidPrebuiltSdfAtlasManifest` | function | 按 family 名校验 manifest。 |
| `PrebuiltSdfAtlasStorageError` | class | 带 `code` 的 atlas 安装错误。 |
| `FontProviderManager`、`LocalizationProviderManager` | class | 渲染器使用的有界 provider 队列。 |
| `BrowserRendererOptions`、`ProfileSceneCreateOptions`、`MasterDataTableLoader`、`MasterDataTableRequest`、`RendererMasterData` | type | 渲染器与 masterdata API。 |
| `ResourceProvider`、`ResourceDescriptor`、`ResourceContext`、`ResourceSource`、`ProvidedResource` | type | 资源 provider 契约。 |
| `FontProvider`、`FontRequest`、`FontContext`、`ProvidedFont`、`FontProviderStats` | type | 字体 provider 契约。 |
| `LocalizationProvider`、`LocalizationRequest`、`LocalizationContext`、`LocalizationProviderStats` | type | 本地化 provider 契约。 |
| `PrebuiltSdfAtlasProvider`、`PrebuiltSdfAtlasManifest`、`PrebuiltSdfAtlasPage`、`PrebuiltSdfAtlasGlyph`、`OriginPrebuiltSdfAtlasPackage`、`PrebuiltSdfAtlasInstallOptions`、`PrebuiltSdfAtlasInstallProgress`、`PrebuiltSdfAtlasPackageStatus`、`PrebuiltSdfAtlasStorageErrorCode` | type | 预生成 atlas API。 |
| `CoreSceneDump`、`CoreSceneDelta`、`CoreInteractionRegion`、`CoreControlBinding`、`StableId` | type | dump 与 delta 结构。 |
| `AuthoringCommand`、`AuthoringDelta`、`AuthoringCategory`、`AuthoringChangeKind`、`AuthoringElementChange`、`AuthoringElementId`、`AuthoringSelection`、`AuthoringPageChange`、`AuthoringPageChangeKind`、`AuthoringCheckpoint`、`GameProfileDocument` | type | 编辑 API。 |
| `RendererFontContract`、`RendererWorkerStats` | type | worker 字体引擎契约与计数。 |

## 许可

AGPL-3.0-only（[LICENSE](https://github.com/empty-sekai/sekai-custom-profile-sdk/blob/main/crates/sekai-profile-wasm/LICENSE)），并附 [LICENSE-EXCEPTION](https://github.com/empty-sekai/sekai-custom-profile-sdk/blob/main/crates/sekai-profile-wasm/LICENSE-EXCEPTION) 中的附加许可。该许可允许网页在最终用户的浏览器中加载未经修改的已发布包，并与网页自身的前端代码链接，而前端代码不因此受 AGPL 约束。修改后的包或渲染器构建，以及在服务器或 Node.js 等任何非浏览器环境中的使用，仍受完整 AGPL 约束，包括网络交互场景下的源代码提供义务。内置 FreeType 及其许可见 [NOTICE](https://github.com/empty-sekai/sekai-custom-profile-sdk/blob/main/crates/sekai-profile-wasm/NOTICE)。
