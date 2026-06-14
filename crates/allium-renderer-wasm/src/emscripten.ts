/**
 * emscripten 运行时模块类型（MODULARIZE + EXPORT_ES6 产物）。
 *
 * 构建产物 `allium_renderer_wasm.js` 默认导出一个工厂函数
 * `createAlliumRenderer(moduleArg?) => Promise<EmscriptenModule>`。
 * 其导出函数集合与 `.cargo/config.toml` 的 EXPORTED_FUNCTIONS /
 * EXPORTED_RUNTIME_METHODS 必须保持一致。
 */

export interface EmscriptenModule {
  /** wasm 线性内存的字节视图。ALLOW_MEMORY_GROWTH 下增长后会被替换，每次用前重读。 */
  HEAPU8: Uint8Array;
  HEAPU32: Uint32Array;

  _malloc(size: number): number;
  _free(ptr: number): void;

  getValue(ptr: number, type: "i32" | "i8" | "i16" | "float" | "double" | "*"): number;
  setValue(ptr: number, value: number, type: "i32" | "i8" | "i16" | "float" | "double" | "*"): void;
  lengthBytesUTF8(str: string): number;
  stringToUTF8(str: string, outPtr: number, maxBytes: number): void;
  UTF8ToString(ptr: number, maxBytes?: number): string;

  cwrap(
    name: string,
    returnType: "number" | "void" | null,
    argTypes: Array<"number" | "string" | "array">,
  ): (...args: number[]) => number;

  // alr_* 导出（直接引用形式，cwrap 是更安全的调用入口）
  _alr_alloc(size: number): number;
  _alr_free(ptr: number, size: number): void;
}

export type EmscriptenModuleFactory = (
  moduleArg?: Partial<EmscriptenModule> & { locateFile?: (path: string, prefix: string) => string },
) => Promise<EmscriptenModule>;
