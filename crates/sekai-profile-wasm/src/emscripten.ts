export type EmscriptenModule = {
  HEAPU8: Uint8Array;
  HEAPU32: Uint32Array;
  _malloc(size: number): number;
  _free(pointer: number): void;
  ccall(
    name: string,
    returnType: "number" | null,
    argumentTypes: Array<"number" | "string">,
    arguments_: Array<number | string>,
  ): number | null;
};

export type EmscriptenModuleFactory = (options?: {
  locateFile?: (path: string) => string;
  printErr?: (...values: unknown[]) => void;
}) => Promise<EmscriptenModule>;
