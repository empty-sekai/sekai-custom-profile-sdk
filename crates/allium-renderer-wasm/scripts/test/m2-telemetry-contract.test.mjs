import assert from "node:assert/strict";
import test from "node:test";

import {
  RendererTelemetry,
  assertTelemetryPrivacy,
  normalizeGpuTimerResult,
} from "../../src/telemetry/rendererTelemetry.ts";

function sample(overrides = {}) {
  return {
    frameId: 1,
    gpuEpoch: 1,
    timingsMs: { coreDynamic: 0.2, gpuSubmitCpu: 0.1, gpuTime: null, total: 0.5 },
    workload: { layers: 100, glyphInstances: 191, dirtyBytes: 8, uploadedBytes: 8 },
    recovery: { contextLosses: 0, restoreMs: null },
    ...overrides,
  };
}

test("Off mode stores and serializes no frame samples", () => {
  const telemetry = new RendererTelemetry({ level: "off" });
  telemetry.record(sample());
  assert.equal(telemetry.snapshot(), null);
  assert.equal(telemetry.stats().recordedFrames, 0);
});

test("Summary mode reports bounded aggregates with explicit null timings", () => {
  const telemetry = new RendererTelemetry({ level: "summary", summaryEvery: 2 });
  telemetry.record(sample());
  telemetry.record(sample({ frameId: 2, timingsMs: { coreDynamic: 0.4, gpuSubmitCpu: 0.2, gpuTime: null, total: 0.9 } }));
  const snapshot = telemetry.snapshot();
  assert.equal(snapshot.level, "summary");
  assert.equal(snapshot.samples, undefined);
  assert.equal(snapshot.summary.total.samples, 2);
  assert.equal(snapshot.last.timingsMs.gpuTime, null);
  assertTelemetryPrivacy(snapshot);
});

test("Trace mode keeps at most 240 sanitized raw samples", () => {
  const telemetry = new RendererTelemetry({ level: "trace", maxSamples: 240 });
  for (let frameId = 0; frameId < 300; frameId += 1) telemetry.record(sample({ frameId }));
  const snapshot = telemetry.snapshot();
  assert.equal(snapshot.samples.length, 240);
  assert.equal(snapshot.samples[0].frameId, 60);
  assert.equal(snapshot.samples.at(-1).frameId, 299);
  assertTelemetryPrivacy(snapshot);
});

test("Trace retention remains bounded when a caller requests an excessive limit", () => {
  const telemetry = new RendererTelemetry({ level: "trace", maxSamples: Number.MAX_SAFE_INTEGER });
  for (let frameId = 0; frameId < 300; frameId += 1) telemetry.record(sample({ frameId }));
  assert.equal(telemetry.snapshot().samples.length, 240);
  assert.equal(telemetry.stats().droppedFrames, 60);
});

test("Invalid trace limits fall back to the bounded default", () => {
  const telemetry = new RendererTelemetry({ level: "trace", maxSamples: Number.NaN });
  for (let frameId = 0; frameId < 300; frameId += 1) telemetry.record(sample({ frameId }));
  assert.equal(telemetry.snapshot().samples.length, 240);
});

test("privacy validator rejects raw content and stable identity at every level", () => {
  for (const forbidden of [
    { rawText: "secret" },
    { userId: "123" },
    { profileSeq: 2 },
    { documentDigest: "deadbeef" },
    { nested: { fontPath: "C:/secret.ttf" } },
  ]) {
    assert.throws(() => assertTelemetryPrivacy(forbidden), /forbidden telemetry field/i);
  }
});

test("disjoint or unavailable GPU timer is null, never a fake zero", () => {
  assert.equal(normalizeGpuTimerResult(1_500_000, false), 1.5);
  assert.equal(normalizeGpuTimerResult(0, false), 0);
  assert.equal(normalizeGpuTimerResult(1_500_000, true), null);
  assert.equal(normalizeGpuTimerResult(undefined, false), null);
});
