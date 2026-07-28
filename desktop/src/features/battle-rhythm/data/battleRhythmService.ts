import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_BATTLE_RHYTHM_EVENT,
  KIND_BATTLE_RHYTHM_REVISION,
  KIND_BATTLE_RHYTHM_SOURCE,
} from "@/shared/constants/kinds";
import {
  buildCalendarEvent,
  buildRevisionEvents,
  buildSourceEvent,
  parseRelayCalendarEvent,
  parseRelaySourceEvent,
  parseRelayRevisionChunk,
  revisionManifestHash,
} from "../domain/eventCodec";
import {
  parseBattleRhythmEvent,
  parseBattleRhythmRevision,
  parseBattleRhythmSource,
  type BattleRhythmEvent,
  type BattleRhythmRevision,
  type BattleRhythmSource,
} from "../domain/contracts";

export type BattleRhythmRange = Readonly<{ start: string; end: string }>;
type Publisher = Pick<typeof relayClient, "publishEvent" | "fetchEvents">;
type RelayReader = Pick<typeof relayClient, "fetchEvents">;
function newest(events: RelayEvent[]): RelayEvent[] {
  return Array.from(
    events
      .reduce((all, event) => {
        const d = event.tags.find((tag) => tag[0] === "d")?.[1];
        const key = `${event.kind}:${d ?? event.id}`;
        const prior = all.get(key);
        if (!prior || event.created_at > prior.created_at) all.set(key, event);
        return all;
      }, new Map<string, RelayEvent>())
      .values(),
  );
}
export type ImportRevisionInput = Readonly<{
  ownerPubkey: string;
  source: BattleRhythmSource;
  revision: BattleRhythmRevision;
  events: readonly BattleRhythmEvent[];
  priorSourceCreatedAt?: number;
  priorEventCreatedAt?: Readonly<Record<string, number>>;
}>;
async function publish(
  publisher: Publisher,
  event: RelayEvent,
): Promise<RelayEvent> {
  return publisher.publishEvent(
    event,
    "Timed out persisting Battle Rhythm data.",
    "Failed to persist Battle Rhythm data.",
  );
}
export async function fetchBattleRhythm(
  ownerPubkey: string,
  range: BattleRhythmRange,
  reader: RelayReader = relayClient,
): Promise<
  Readonly<{
    sources: readonly BattleRhythmSource[];
    events: readonly BattleRhythmEvent[];
  }>
> {
  const [sourceEvents, calendarEvents, revisionEvents] = await Promise.all([
    reader.fetchEvents({
      kinds: [KIND_BATTLE_RHYTHM_SOURCE],
      authors: [ownerPubkey],
      limit: 500,
    }),
    reader.fetchEvents({
      kinds: [KIND_BATTLE_RHYTHM_EVENT],
      authors: [ownerPubkey],
      limit: 2000,
    }),
    reader.fetchEvents({
      kinds: [KIND_BATTLE_RHYTHM_REVISION],
      authors: [ownerPubkey],
      limit: 5000,
    }),
  ]);
  const chunks = revisionEvents
    .map(parseRelayRevisionChunk)
    .filter((chunk): chunk is NonNullable<typeof chunk> => chunk !== null);
  const eligibleRevisionKeys = new Set<string>();
  const revisionsByKey = new Map<string, BattleRhythmRevision>();
  const chunkGroups = new Map<string, typeof chunks>();
  for (const chunk of chunks) {
    const group = chunkGroups.get(chunk.revisionId) ?? [];
    group.push(chunk);
    chunkGroups.set(chunk.revisionId, group);
  }
  for (const [revisionId, group] of chunkGroups) {
    const first = group[0];
    if (
      !first ||
      group.length !== first.chunkCount ||
      group.some(
        (chunk) =>
          chunk.chunkCount !== first.chunkCount ||
          chunk.sourceId !== first.sourceId ||
          chunk.manifestHash !== first.manifestHash,
      )
    )
      continue;
    const ordered = [...group].sort((a, b) => a.chunkIndex - b.chunkIndex);
    if (ordered.some((chunk, index) => chunk.chunkIndex !== index)) continue;
    try {
      const revision = parseBattleRhythmRevision({
        schemaVersion: 1,
        id: revisionId,
        sourceId: first.sourceId,
        priorRevisionId: first.priorRevisionId,
        importedAt: first.importedAt,
        changes: ordered.flatMap((chunk) => chunk.changes),
      });
      if ((await revisionManifestHash(revision)) === first.manifestHash)
        eligibleRevisionKeys.add(`${first.sourceId}:${revisionId}`);
      revisionsByKey.set(`${first.sourceId}:${revisionId}`, revision);
    } catch {
      /* invalid chunks are ineligible */
    }
  }
  const sources = newest(sourceEvents)
    .map(parseRelaySourceEvent)
    .filter((x): x is BattleRhythmSource => x !== null)
    .filter((source) =>
      eligibleRevisionKeys.has(`${source.id}:${source.revisionId}`),
    );
  const removedEventIds = new Set(
    sources.flatMap((source) => {
      const revision = revisionsByKey.get(`${source.id}:${source.revisionId}`);
      return (
        revision?.changes
          .filter((change) => change.kind === "removed")
          .map((change) => change.before.id) ?? []
      );
    }),
  );
  const events = newest(calendarEvents)
    .map(parseRelayCalendarEvent)
    .filter((x): x is BattleRhythmEvent => x !== null)
    .filter((event) => !removedEventIds.has(event.id))
    .filter(
      (event) =>
        Date.parse(event.start) < Date.parse(range.end) &&
        Date.parse(event.end) > Date.parse(range.start),
    );
  return Object.freeze({
    sources: Object.freeze(sources),
    events: Object.freeze(events),
  });
}
export async function publishManualEvent(
  input: BattleRhythmEvent,
  priorCreatedAt?: number,
): Promise<RelayEvent> {
  const event = parseBattleRhythmEvent(input);
  if (event.ownership.kind !== "manual")
    throw new Error("Manual publication requires manual ownership");
  return publish(relayClient, await buildCalendarEvent(event, priorCreatedAt));
}
export async function applyImportRevision(
  input: ImportRevisionInput,
  publisher: Publisher = relayClient,
): Promise<void> {
  const source = parseBattleRhythmSource(input.source);
  const revision = parseBattleRhythmRevision(input.revision);
  if (source.id !== revision.sourceId || source.revisionId !== revision.id)
    throw new Error("Source active revision must match imported revision");
  const events = input.events.map(parseBattleRhythmEvent);
  for (const event of events)
    if (
      event.ownership.kind !== "source" ||
      event.ownership.sourceId !== source.id ||
      event.ownership.revisionId !== revision.id
    )
      throw new Error(
        "Import may only replace events owned by its source revision",
      );
  const existing = await publisher.fetchEvents({
    kinds: [KIND_BATTLE_RHYTHM_EVENT],
    authors: [input.ownerPubkey],
    limit: 2000,
  });
  const existingById = new Map(
    newest(existing)
      .map((head) => ({
        id: head.tags.find((tag) => tag[0] === "d")?.[1],
        parsed: parseRelayCalendarEvent(head),
      }))
      .filter(
        (head): head is { id: string; parsed: BattleRhythmEvent | null } =>
          Boolean(head.id),
      )
      .map((head) => [head.id, head.parsed]),
  );
  for (const event of events) {
    const prior = existingById.get(event.id);
    if (
      existingById.has(event.id) &&
      (!prior ||
        prior.ownership.kind === "manual" ||
        prior.ownership.sourceId !== source.id)
    )
      throw new Error(
        "Import may not replace a manual or other-source event head",
      );
  }
  const heads = await Promise.all(
    events.map((event) =>
      buildCalendarEvent(event, input.priorEventCreatedAt?.[event.id]),
    ),
  );
  const chunks = await buildRevisionEvents(revision); // Build and validate every immutable chunk before any relay mutation.
  for (const event of heads) await publish(publisher, event);
  for (const chunk of chunks) await publish(publisher, chunk);
  await publish(
    publisher,
    await buildSourceEvent(source, input.priorSourceCreatedAt),
  );
}
