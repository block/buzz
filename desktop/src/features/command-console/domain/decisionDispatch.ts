import type { DecisionExecutionStatus } from "./decisionExecutionStore";

export function buildCommandDirectionMessage(input: {
  directionId: string;
  decision: string;
  direction: string;
}): string {
  return [
    `CO DIRECTION ${input.directionId}`,
    "",
    `Decision: ${input.decision}`,
    `Direction: ${input.direction}`,
    "",
    "Begin work immediately. Coordinate the relevant command advisers and use the connected systems needed to carry out this direction. This direction is the approval for those actions; do not request a second approval.",
    "",
    `Report progress in this DM using the exact identifier ${input.directionId}:`,
    `CO DIRECTION ${input.directionId} — IN PROGRESS`,
    `CO DIRECTION ${input.directionId} — COMPLETE`,
    `CO DIRECTION ${input.directionId} — BLOCKED`,
    `CO DIRECTION ${input.directionId} — FAILED`,
    "Add only a short result or reason after the status line.",
  ].join("\n");
}

const STATUS_MAP: Readonly<Record<string, DecisionExecutionStatus>> = {
  "IN PROGRESS": "in_progress",
  COMPLETE: "completed",
  BLOCKED: "blocked",
  FAILED: "failed",
};

export function parseCommandDirectionStatus(
  content: string,
  directionId: string,
): { status: DecisionExecutionStatus; statusText: string } | null {
  const firstLine =
    content
      .split(/\r?\n/, 1)[0]
      ?.trim()
      .replace(/^[*_`~]+|[*_`~]+$/g, "") ?? "";
  const prefix = `CO DIRECTION ${directionId}`;
  if (!firstLine.toLocaleUpperCase().startsWith(prefix.toLocaleUpperCase())) {
    return null;
  }
  const rawStatus = firstLine
    .slice(prefix.length)
    .replace(/^[\s\u2014-]+/, "")
    .trim()
    .toLocaleUpperCase();
  const status = STATUS_MAP[rawStatus];
  if (!status) return null;
  const detail = content.split(/\r?\n/).slice(1).join(" ").trim();
  return {
    status,
    statusText:
      detail ||
      {
        in_progress: "Chief of Staff is working.",
        completed: "Direction completed.",
        blocked: "Direction is blocked.",
        failed: "Direction failed.",
        queued: "Direction queued.",
        stalled: "Direction stalled.",
      }[status],
  };
}
