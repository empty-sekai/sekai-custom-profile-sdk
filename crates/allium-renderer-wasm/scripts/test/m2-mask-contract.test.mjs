import test from "node:test";
import assert from "node:assert/strict";

import { contiguousSlotRanges } from "../../src/gpu/slotRanges.ts";

test("subtree mask patches coalesce adjacent DFS slots", () => {
  assert.deepEqual(contiguousSlotRanges([7, 3, 4, 5, 7, 10]), [
    { start: 3, end: 6 },
    { start: 7, end: 8 },
    { start: 10, end: 11 },
  ]);
});

test("empty mask patch produces no GPU upload range", () => {
  assert.deepEqual(contiguousSlotRanges([]), []);
});
