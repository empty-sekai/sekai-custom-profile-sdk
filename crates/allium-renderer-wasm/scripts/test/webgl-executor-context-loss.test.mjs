import assert from "node:assert/strict";
import test from "node:test";

import { WebglSemanticCommandExecutor } from "../../dist/gpu/webglSemanticCommandExecutor.js";

// A WebGL2 context stand-in: every call succeeds, and once the context is
// lost, queries return null exactly as a lost WebGL context does.
function webglContext() {
  const state = { lost: false };
  const methods = {
    isContextLost: () => state.lost,
    getShaderParameter: () => (state.lost ? null : true),
    getProgramParameter: () => (state.lost ? null : true),
    getParameter: (name) => {
      if (state.lost) return null;
      return name === "VIEWPORT" ? new Int32Array([0, 0, 1830, 812]) : null;
    },
    checkFramebufferStatus: () => "FRAMEBUFFER_COMPLETE",
  };
  const gl = new Proxy(methods, {
    get(target, property) {
      if (property in target) return target[property];
      if (typeof property === "string" && /^[A-Z0-9_]+$/.test(property)) return property;
      return () => (state.lost ? null : {});
    },
  });
  return { gl, state };
}

test("drawing into a lost context reports the loss instead of a null viewport", () => {
  const { gl, state } = webglContext();
  const executor = new WebglSemanticCommandExecutor(gl);
  assert.equal(executor.draw().drawCalls, 0);
  state.lost = true;
  assert.throws(() => executor.draw(), { name: "Error", message: "WebGL context is lost" });
});

test("restoring onto a context that is still lost reports the loss, not a shader error", () => {
  const { gl, state } = webglContext();
  state.lost = true;
  assert.throws(() => new WebglSemanticCommandExecutor(gl), { name: "Error", message: "WebGL context is lost" });
});
