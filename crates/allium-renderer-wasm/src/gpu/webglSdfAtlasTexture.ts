import type { SdfAtlas } from "../fontSdfAtlas.js";

export class WebglSdfAtlasTexture {
  private textureValue: WebGLTexture;
  private contractId: string | null = null;
  private revisions = new Map<number, number>();
  private allocated = false;

  constructor(private readonly gl: WebGL2RenderingContext) {
    const texture = gl.createTexture();
    if (!texture) throw new Error("SDF atlas texture allocation failed");
    this.textureValue = texture;
  }

  texture(): WebGLTexture {
    return this.textureValue;
  }

  setAtlas(atlas: SdfAtlas): void {
    if (this.contractId === atlas.contractId) return;
    this.contractId = atlas.contractId;
    this.revisions.clear();
    this.allocated = false;
    this.gl.deleteTexture(this.textureValue);
    const texture = this.gl.createTexture();
    if (!texture) throw new Error("SDF atlas texture reallocation failed");
    this.textureValue = texture;
  }

  async uploadUpdates(atlas: SdfAtlas): Promise<{ bytes: number; rects: number }> {
    this.setAtlas(atlas);
    const updates = await atlas.pageUpdates(this.revisions);
    if (updates.length === 0) return { bytes: 0, rects: 0 };
    const gl = this.gl;
    gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, 0);
    gl.bindTexture(gl.TEXTURE_2D_ARRAY, this.textureValue);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    if (!this.allocated) {
      const supportedPages = gl.getParameter(gl.MAX_ARRAY_TEXTURE_LAYERS) as number;
      if (atlas.depth > supportedPages) {
        throw new Error(`atlas requires ${atlas.depth} GPU layers but this device supports ${supportedPages}`);
      }
      gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.texStorage3D(gl.TEXTURE_2D_ARRAY, 1, gl.R8, atlas.width, atlas.height, atlas.depth);
      this.allocated = true;
    }
    let bytes = 0;
    let rects = 0;
    for (const update of updates) {
      if (update.page >= atlas.depth) throw new Error(`atlas page ${update.page} exceeds allocated depth ${atlas.depth}`);
      if (update.fullUpload) {
        gl.texSubImage3D(gl.TEXTURE_2D_ARRAY, 0, 0, 0, update.page, atlas.width, atlas.height, 1, gl.RED, gl.UNSIGNED_BYTE, update.pixels);
        bytes += update.pixels.byteLength;
        rects += 1;
      } else {
        for (const rect of update.dirtyRects) {
          gl.texSubImage3D(gl.TEXTURE_2D_ARRAY, 0, rect.x, rect.y, update.page, rect.width, rect.height, 1, gl.RED, gl.UNSIGNED_BYTE, rect.pixels);
          bytes += rect.pixels.byteLength;
          rects += 1;
        }
      }
      this.revisions.set(update.page, update.revision);
    }
    return { bytes, rects };
  }

  destroy(): void {
    this.gl.deleteTexture(this.textureValue);
  }
}
