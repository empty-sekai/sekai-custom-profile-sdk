export function packPreviewTransformsForTexture(transforms: Float32Array, width: number): Float32Array {
  if (!Number.isInteger(width) || width < 1 || transforms.length !== width * 8) {
    throw new Error(`invalid preview transform buffer ${transforms.length} for width ${width}`);
  }
  const packed = new Float32Array(transforms.length);
  for (let slot = 0; slot < width; slot += 1) {
    const source = slot * 8;
    packed.set(transforms.subarray(source, source + 4), slot * 4);
    packed.set(transforms.subarray(source + 4, source + 8), (width + slot) * 4);
  }
  return packed;
}
