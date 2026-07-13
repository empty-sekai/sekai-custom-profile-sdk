export type SlotRange = { start: number; end: number };

export function contiguousSlotRanges(slots: number[]): SlotRange[] {
  const sorted = [...new Set(slots)].sort((left, right) => left - right);
  const ranges: SlotRange[] = [];
  for (const slot of sorted) {
    const last = ranges.at(-1);
    if (last && slot === last.end) {
      last.end += 1;
    } else {
      ranges.push({ start: slot, end: slot + 1 });
    }
  }
  return ranges;
}
