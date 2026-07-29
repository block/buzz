import type { BattleRhythmEvent } from "./contracts";

export function applyLocalAdjustments(
  events: readonly BattleRhythmEvent[],
): readonly BattleRhythmEvent[] {
  const adjustedIds = new Set(
    events
      .filter(
        (event) =>
          event.ownership.kind === "manual" && event.parentActivityId !== null,
      )
      .map((event) => event.parentActivityId as string),
  );
  return events.filter((event) => !adjustedIds.has(event.id));
}
