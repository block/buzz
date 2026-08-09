import type { ImportRevisionInput } from "./battleRhythmService";
import {
  parseBattleRhythmEvent,
  parseBattleRhythmRevision,
  parseBattleRhythmSource,
  type BattleRhythmChange,
  type BattleRhythmRevision,
  type BattleRhythmSource,
} from "../domain/contracts";
import { reconstructSourceRevision } from "../domain/revisionState";

export type RollbackPreview = Readonly<{
  input: ImportRevisionInput;
  targetRevisionId: string;
  added: number;
  changed: number;
  removed: number;
}>;

export function buildRollbackPreview({
  ownerPubkey,
  source,
  revisions,
  targetRevisionId,
  revisionId,
  importedAt,
}: Readonly<{
  ownerPubkey: string;
  source: BattleRhythmSource;
  revisions: readonly BattleRhythmRevision[];
  targetRevisionId: string;
  revisionId: string;
  importedAt: string;
}>): RollbackPreview {
  if (targetRevisionId === source.revisionId)
    throw new Error("The selected revision is already active.");
  const current = reconstructSourceRevision(
    revisions,
    source.id,
    source.revisionId,
  );
  const target = reconstructSourceRevision(
    revisions,
    source.id,
    targetRevisionId,
  );
  const changes: BattleRhythmChange[] = [];
  const events = [];
  let added = 0;
  let changed = 0;
  let removed = 0;
  for (const [id, before] of current) {
    if (!target.has(id)) {
      changes.push({ kind: "removed", before });
      removed += 1;
    }
  }
  for (const [id, targetEvent] of target) {
    const after = parseBattleRhythmEvent({
      ...targetEvent,
      ownership: {
        kind: "source",
        sourceId: source.id,
        revisionId,
        sourceLocation:
          targetEvent.ownership.kind === "source"
            ? targetEvent.ownership.sourceLocation
            : `rollback:${id}`,
      },
    });
    const before = current.get(id);
    if (before) {
      changes.push({ kind: "changed", before, after });
      changed += 1;
    } else {
      changes.push({ kind: "added", after });
      added += 1;
    }
    events.push(after);
  }
  const revision = parseBattleRhythmRevision({
    schemaVersion: 1,
    id: revisionId,
    sourceId: source.id,
    priorRevisionId: source.revisionId,
    importedAt,
    changes,
  });
  return Object.freeze({
    targetRevisionId,
    added,
    changed,
    removed,
    input: Object.freeze({
      ownerPubkey,
      source: parseBattleRhythmSource({
        ...source,
        revisionId,
        priorRevisionId: source.revisionId,
        importedAt,
        status: "approved",
        sourceReference: `rollback:${targetRevisionId}`,
      }),
      revision,
      events: Object.freeze(events),
    }),
  });
}
