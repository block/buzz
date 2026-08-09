import {
  parseBattleRhythmEvent,
  type BattleRhythmEvent,
  type BattleRhythmRevision,
} from "./contracts";

function same(left: BattleRhythmEvent, right: BattleRhythmEvent) {
  return JSON.stringify(left) === JSON.stringify(right);
}

export function reconstructSourceRevision(
  revisions: readonly BattleRhythmRevision[],
  sourceId: string,
  revisionId: string,
): ReadonlyMap<string, BattleRhythmEvent> {
  const byId = new Map(
    revisions
      .filter((revision) => revision.sourceId === sourceId)
      .map((revision) => [revision.id, revision]),
  );
  const memo = new Map<string, Map<string, BattleRhythmEvent>>();
  const visiting = new Set<string>();
  function visit(id: string): Map<string, BattleRhythmEvent> {
    const priorResult = memo.get(id);
    if (priorResult) return new Map(priorResult);
    if (visiting.has(id)) throw new Error("Revision history contains a cycle.");
    const revision = byId.get(id);
    if (!revision) throw new Error("Revision history is incomplete.");
    visiting.add(id);
    const state = revision.priorRevisionId
      ? visit(revision.priorRevisionId)
      : new Map<string, BattleRhythmEvent>();
    for (const change of revision.changes) {
      const before = change.kind === "added" ? undefined : change.before;
      const after = change.kind === "removed" ? undefined : change.after;
      for (const event of [before, after]) {
        if (!event) continue;
        if (
          event.ownership.kind !== "source" ||
          event.ownership.sourceId !== sourceId
        )
          throw new Error("Revision contains an event from another source.");
      }
      if (change.kind === "added") {
        if (state.has(change.after.id))
          throw new Error("Revision adds an existing event.");
        state.set(change.after.id, parseBattleRhythmEvent(change.after));
      } else if (change.kind === "changed") {
        const current = state.get(change.before.id);
        if (
          change.before.id !== change.after.id ||
          !current ||
          !same(current, change.before)
        )
          throw new Error("Revision change does not match its parent.");
        state.set(change.after.id, parseBattleRhythmEvent(change.after));
      } else {
        const current = state.get(change.before.id);
        if (!current || !same(current, change.before))
          throw new Error("Revision removal does not match its parent.");
        state.delete(change.before.id);
      }
    }
    visiting.delete(id);
    memo.set(id, new Map(state));
    return state;
  }
  return visit(revisionId);
}
