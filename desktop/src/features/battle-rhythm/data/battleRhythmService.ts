import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_BATTLE_RHYTHM_EVENT,
  KIND_BATTLE_RHYTHM_SOURCE,
} from "@/shared/constants/kinds";
import {
  buildCalendarEvent,
  buildRevisionEvents,
  buildSourceEvent,
  parseRelayCalendarEvent,
  parseRelaySourceEvent,
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
type Publisher = Pick<typeof relayClient, "publishEvent">;
export type ImportRevisionInput = Readonly<{
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
): Promise<
  Readonly<{
    sources: readonly BattleRhythmSource[];
    events: readonly BattleRhythmEvent[];
  }>
> {
  const [sourceEvents, calendarEvents] = await Promise.all([
    relayClient.fetchEvents({
      kinds: [KIND_BATTLE_RHYTHM_SOURCE],
      authors: [ownerPubkey],
      limit: 500,
    }),
    relayClient.fetchEvents({
      kinds: [KIND_BATTLE_RHYTHM_EVENT],
      authors: [ownerPubkey],
      limit: 2000,
    }),
  ]);
  const newest = (events: RelayEvent[]) =>
    Array.from(
      events
        .reduce((all, event) => {
          const d = event.tags.find((t) => t[0] === "d")?.[1];
          const key = `${event.kind}:${d ?? event.id}`;
          const prior = all.get(key);
          if (!prior || event.created_at > prior.created_at)
            all.set(key, event);
          return all;
        }, new Map<string, RelayEvent>())
        .values(),
    );
  const sources = newest(sourceEvents)
    .map(parseRelaySourceEvent)
    .filter((x): x is BattleRhythmSource => x !== null);
  const events = newest(calendarEvents)
    .map(parseRelayCalendarEvent)
    .filter((x): x is BattleRhythmEvent => x !== null)
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
