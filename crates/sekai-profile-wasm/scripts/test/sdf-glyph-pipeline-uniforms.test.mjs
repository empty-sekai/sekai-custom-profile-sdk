import assert from "node:assert/strict";
import test from "node:test";

import "./register-typescript.mjs";

const { SDF_GLYPH_FLOATS_PER_INSTANCE, WebglSdfGlyphPipeline } = await import("../../src/gpu/webglSdfGlyphPipeline.ts");

function recordingContext() {
  const log = { lookups: 0, uniforms: [] };
  const methods = {
    getShaderParameter: () => true,
    getProgramParameter: () => true,
    getUniformLocation: (_program, name) => {
      log.lookups += 1;
      return { name };
    },
    uniform1i: (location, value) => log.uniforms.push([location?.name ?? null, value]),
    uniform1f: (location, value) => log.uniforms.push([location?.name ?? null, value]),
  };
  const gl = new Proxy(methods, {
    get(target, property) {
      if (property in target) return target[property];
      if (typeof property === "string" && /^[A-Z0-9_]+$/.test(property)) return property;
      return () => ({});
    },
  });
  return { gl, log };
}

test("glyph pipeline resolves its uniform locations once, not per draw", () => {
  const { gl, log } = recordingContext();
  const pipeline = new WebglSdfGlyphPipeline(gl);
  pipeline.upload("batch", new Float32Array(SDF_GLYPH_FLOATS_PER_INSTANCE));
  const lookupsBeforeDraw = log.lookups;
  const state = {};
  const mask = {};
  pipeline.draw("batch", state, mask, 4);
  pipeline.draw("batch", state, mask, 4);
  assert.equal(log.lookups, lookupsBeforeDraw);
  const drawUniforms = log.uniforms.slice(-8);
  assert.deepEqual(drawUniforms, [
    ["u_atlas", 0],
    ["u_layerState", 1],
    ["u_layerStateWidth", 4],
    ["u_renderMask", 2],
    ["u_commandMask", 3],
    ["u_commandState", 4],
    ["u_commandWidth", 4],
    ["u_previewTransform", 5],
  ]);
});
