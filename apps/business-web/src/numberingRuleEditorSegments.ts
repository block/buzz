import type { NumberingRule, NumberingSegment } from "./api";

export type EditableNumberingSegment = {
  key: string;
  segment: NumberingSegment;
};

let nextSegmentKey = 0;

function editableSegment(segment: NumberingSegment): EditableNumberingSegment {
  nextSegmentKey += 1;
  return {
    key: `numbering-segment-${nextSegmentKey}`,
    segment: { ...segment },
  };
}

export function createEditableSegments(segments: NumberingSegment[]) {
  return segments.map(editableSegment);
}

export function replaceEditableSegment(
  rows: EditableNumberingSegment[],
  index: number,
  segment: NumberingSegment,
) {
  return rows.map((row, at) =>
    at === index ? { ...row, segment } : row,
  );
}

export function moveEditableSegment(
  rows: EditableNumberingSegment[],
  index: number,
  offset: -1 | 1,
) {
  const target = index + offset;
  if (target < 0 || target >= rows.length) return rows;
  const next = [...rows];
  [next[index], next[target]] = [next[target], next[index]];
  return next;
}

export function removeEditableSegment(
  rows: EditableNumberingSegment[],
  index: number,
) {
  return rows.filter((_, at) => at !== index);
}

export function appendEditableSegment(
  rows: EditableNumberingSegment[],
  segment: NumberingSegment,
) {
  return [...rows, editableSegment(segment)];
}

export function changeEditableScope(
  rows: EditableNumberingSegment[],
  nextScope: NumberingRule["scopeDimension"],
) {
  const withoutScope = rows.filter((row) => row.segment.type !== "scope");
  if (nextScope === "global") return withoutScope;
  const sequenceAt = withoutScope.findIndex(
    (row) => row.segment.type === "sequence",
  );
  const at = sequenceAt < 0 ? withoutScope.length : sequenceAt;
  return [
    ...withoutScope.slice(0, at),
    editableSegment({ type: "scope" }),
    ...withoutScope.slice(at),
  ];
}
