import assert from "node:assert/strict";
import test from "node:test";

import { WebglSemanticCommandExecutor } from "../../dist/gpu/webglSemanticCommandExecutor.js";

// A WebGL2 context stand-in that tracks live objects and fails the Nth shader
// compile or program link.
function countingContext({ failCompileAt = 0, failLinkAt = 0, infoLog = "" } = {}) {
  const live = new Set();
  let compiles = 0;
  let links = 0;
  const create = (kind) => () => {
    const object = { kind };
    live.add(object);
    return object;
  };
  const release = (object) => { live.delete(object); };
  const methods = {
    createShader: create("shader"),
    createProgram: create("program"),
    createTexture: create("texture"),
    createVertexArray: create("vertex array"),
    createBuffer: create("buffer"),
    createFramebuffer: create("framebuffer"),
    deleteShader: release,
    deleteProgram: release,
    deleteTexture: release,
    deleteVertexArray: release,
    deleteBuffer: release,
    deleteFramebuffer: release,
    getShaderParameter: () => (compiles += 1) !== failCompileAt,
    getProgramParameter: () => (links += 1) !== failLinkAt,
    getShaderInfoLog: () => infoLog,
    getProgramInfoLog: () => infoLog,
    isContextLost: () => false,
  };
  const gl = new Proxy(methods, {
    get(target, property) {
      if (property in target) return target[property];
      if (typeof property === "string" && /^[A-Z0-9_]+$/.test(property)) return property;
      return () => undefined;
    },
  });
  return { gl, live };
}

test("a successful executor releases every object it created", () => {
  const { gl, live } = countingContext();
  new WebglSemanticCommandExecutor(gl).destroy();
  assert.deepEqual([...live].map((object) => object.kind), []);
});

for (let failCompileAt = 1; failCompileAt <= 10; failCompileAt += 1) {
  test(`a failed shader compile ${failCompileAt} releases the objects created so far`, () => {
    const { gl, live } = countingContext({ failCompileAt, infoLog: "ERROR: 0:1: syntax error" });
    assert.throws(() => new WebglSemanticCommandExecutor(gl), /syntax error/);
    assert.deepEqual([...live].map((object) => object.kind), []);
  });
}

for (let failLinkAt = 1; failLinkAt <= 5; failLinkAt += 1) {
  test(`a failed program link ${failLinkAt} releases the objects created so far`, () => {
    const { gl, live } = countingContext({ failLinkAt, infoLog: "link error" });
    assert.throws(() => new WebglSemanticCommandExecutor(gl), /link error/);
    assert.deepEqual([...live].map((object) => object.kind), []);
  });
}

test("an empty driver log still names the failing shader stage", () => {
  const { gl } = countingContext({ failCompileAt: 2 });
  assert.throws(() => new WebglSemanticCommandExecutor(gl), /fragment shader compile failed/);
  const link = countingContext({ failLinkAt: 1 });
  assert.throws(() => new WebglSemanticCommandExecutor(link.gl), /program link failed/);
});
