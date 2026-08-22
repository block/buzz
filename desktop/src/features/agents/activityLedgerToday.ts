import type { RelayEvent } from "@/shared/api/types";
import { decryptObserverEvent } from "@/shared/api/tauriObserver";
import { applyValidatedJournalAuthority } from "./activityLedgerAuthority";
import type { ValidatedJournalAuthorityArtifact } from "./activityLedgerAuthority";
import {
  buildTodayActivitySurface,
  normalizeActivityEvents,
  type NormalizedActivityEvent,
  type TodayActivityJournal,
  type TodayActivitySurface,
} from "./activityLedger";
import type { ObserverEvent } from "./ui/agentSessionTypes";

export type ActivityLedgerAgentIdentity = {
  pubkey: string;
  name: string;
};

export const TODAY_SNAPSHOT_SURFACE_MAX_BYTES = 6 * 1024 * 1024;
const TODAY_SNAPSHOT_MAX_EVENTS_PER_JOURNAL = 100;
const TODAY_ARCHIVE_MAX_EVENTS_PER_JOURNAL = 300;
const TODAY_SNAPSHOT_MAX_SUMMARY_CHARS = 4_096;
const TODAY_SNAPSHOT_MAX_EVENT_DETAIL_CHARS = 8_192;
const TODAY_ARCHIVE_ENVELOPE_OVERLAP_SECONDS = 2;

export type TodaySnapshotProjection = {
  bounded: boolean;
  maxBytes: number;
  originalJournals: number;
  includedJournals: number;
  omittedJournals: number;
  omittedEvents: number;
  textFieldsTruncated: number;
};

export type BoundedTodayActivitySurface = TodayActivitySurface & {
  snapshotProjection: TodaySnapshotProjection;
};

function truncateSnapshotText(value: string, maxChars: number) {
  if (value.length <= maxChars) return { value, truncated: false };
  return { value: `${value.slice(0, maxChars - 1)}…`, truncated: true };
}

function selectSnapshotEvents(
  journal: TodayActivityJournal,
  maxEvents: number,
) {
  if (journal.events.length <= maxEvents) return journal.events;
  const selected = new Set<number>();
  const add = (index: number | undefined) => {
    if (
      index !== undefined &&
      index >= 0 &&
      index < journal.events.length &&
      selected.size < maxEvents
    ) {
      selected.add(index);
    }
  };
  const addMatching = (
    predicate: (event: NormalizedActivityEvent) => boolean,
  ) => {
    for (let index = 0; index < journal.events.length; index += 1) {
      if (predicate(journal.events[index])) add(index);
    }
  };

  // Verification sources are capped at 256 in the owner-authority contract.
  // Preserve those sources, their correlation root, and terminal truth before
  // spending the remaining projection budget on ordinary transcript detail.
  add(0);
  addMatching(
    (event) =>
      event.provenance.observerKind === "owner_verification" ||
      event.provenance.sourceKind === 24201,
  );
  addMatching((event) => event.proofState === "VERIFIED");
  add(
    journal.events.findIndex(
      (event) =>
        event.provenance.triggeringEventIds.includes(journal.correlationId) ||
        event.toolCallId === journal.correlationId ||
        event.messageId === journal.correlationId,
    ),
  );
  addMatching((event) => event.proofState === "FAILED");

  let latestVerificationEvidence = -1;
  for (let index = journal.events.length - 1; index >= 0; index -= 1) {
    const event = journal.events[index];
    if (
      event.provenance.sourceKind !== 24201 &&
      event.provenance.observerKind !== "owner_verification" &&
      (event.category === "turn" ||
        event.category === "tool" ||
        event.category === "permission" ||
        event.category === "status")
    ) {
      latestVerificationEvidence = index;
      break;
    }
  }
  add(latestVerificationEvidence);
  addMatching((event) => event.category === "turn");
  addMatching((event) => event.proofState === "RECEIPTED");

  const latestToolEvents = new Map<string, number>();
  for (let index = 0; index < journal.events.length; index += 1) {
    const event = journal.events[index];
    if (event.category === "tool") {
      latestToolEvents.set(event.toolCallId ?? event.correlationId, index);
    }
  }
  for (const index of latestToolEvents.values()) add(index);

  for (
    let index = journal.events.length - 1;
    index >= 0 && selected.size < maxEvents;
    index -= 1
  ) {
    add(index);
  }
  return [...selected]
    .sort((left, right) => left - right)
    .map((index) => journal.events[index]);
}

function compactSnapshotJournal(
  journal: TodayActivityJournal,
  maxEvents = TODAY_SNAPSHOT_MAX_EVENTS_PER_JOURNAL,
): {
  journal: TodayActivityJournal;
  textFieldsTruncated: number;
} {
  const summary = truncateSnapshotText(
    journal.summary,
    TODAY_SNAPSHOT_MAX_SUMMARY_CHARS,
  );
  const selectedEvents = selectSnapshotEvents(journal, maxEvents);
  let textFieldsTruncated = summary.truncated ? 1 : 0;
  const events = selectedEvents.map((event) => {
    if (event.detail === null) return event;
    const detail = truncateSnapshotText(
      event.detail,
      TODAY_SNAPSHOT_MAX_EVENT_DETAIL_CHARS,
    );
    if (detail.truncated) textFieldsTruncated += 1;
    return detail.truncated ? { ...event, detail: detail.value } : event;
  });
  return {
    journal: {
      ...journal,
      summary: summary.value,
      events,
    },
    textFieldsTruncated,
  };
}

function snapshotSurfaceFromJournals(input: {
  day: string;
  journals: TodayActivityJournal[];
  maxBytes: number;
  originalJournalCount: number;
  originalEventCount: number;
  textFieldsTruncated: number;
}): BoundedTodayActivitySurface {
  const channels = new Map<
    string,
    {
      journalIds: string[];
      agentPubkeys: Set<string>;
      agentNames: Set<string>;
      lastActivityAt: string;
    }
  >();
  for (const journal of input.journals) {
    if (!journal.channelId) continue;
    const bucket = channels.get(journal.channelId) ?? {
      journalIds: [],
      agentPubkeys: new Set<string>(),
      agentNames: new Set<string>(),
      lastActivityAt: journal.endedAt,
    };
    bucket.journalIds.push(journal.id);
    bucket.agentPubkeys.add(journal.agentPubkey);
    bucket.agentNames.add(journal.agentName);
    if (Date.parse(journal.endedAt) > Date.parse(bucket.lastActivityAt)) {
      bucket.lastActivityAt = journal.endedAt;
    }
    channels.set(journal.channelId, bucket);
  }
  const includedEventCount = input.journals.reduce(
    (count, journal) => count + journal.events.length,
    0,
  );
  const omittedJournals = Math.max(
    0,
    input.originalJournalCount - input.journals.length,
  );
  const omittedEvents = Math.max(
    0,
    input.originalEventCount - includedEventCount,
  );
  return {
    day: input.day,
    journals: input.journals,
    channels: [...channels.entries()]
      .map(([channelId, bucket]) => ({
        channelId,
        journalIds: bucket.journalIds,
        agentPubkeys: [...bucket.agentPubkeys],
        agentNames: [...bucket.agentNames],
        lastActivityAt: bucket.lastActivityAt,
      }))
      .sort((left, right) => left.channelId.localeCompare(right.channelId)),
    counts: {
      journals: input.journals.length,
      failed: input.journals.filter((journal) => journal.status === "failed")
        .length,
      inProgress: input.journals.filter(
        (journal) => journal.status === "in_progress",
      ).length,
      claimedWithoutEvidence: input.journals.filter(
        (journal) => journal.claimedCompletionWithoutEvidence,
      ).length,
    },
    snapshotProjection: {
      bounded:
        omittedJournals > 0 ||
        omittedEvents > 0 ||
        input.textFieldsTruncated > 0,
      maxBytes: input.maxBytes,
      originalJournals: input.originalJournalCount,
      includedJournals: input.journals.length,
      omittedJournals,
      omittedEvents,
      textFieldsTruncated: input.textFieldsTruncated,
    },
  };
}

function snapshotSurfaceByteLength(surface: BoundedTodayActivitySurface) {
  return new TextEncoder().encode(JSON.stringify(surface)).byteLength;
}

/**
 * Bound the signed Today projection without letting one large tool output
 * suppress the entire feed. Summaries/details are capped first, then event
 * bodies are removed, and only as a final fallback are the oldest journals
 * omitted. The newest retained journals stay in chronological order.
 */
export function buildBoundedTodayActivitySurface(
  surface: TodayActivitySurface,
  maxBytes = TODAY_SNAPSHOT_SURFACE_MAX_BYTES,
): BoundedTodayActivitySurface {
  const previousProjection = (
    surface as TodayActivitySurface & {
      snapshotProjection?: TodaySnapshotProjection;
    }
  ).snapshotProjection;
  const ordered = [...surface.journals].sort(
    (left, right) =>
      Date.parse(left.startedAt) - Date.parse(right.startedAt) ||
      left.id.localeCompare(right.id),
  );
  const compacted = ordered.map((journal) => compactSnapshotJournal(journal));
  const retainedOriginalEventCount = ordered.reduce(
    (count, journal) =>
      count + Math.max(journal.eventCount, journal.events.length),
    0,
  );
  const originalJournalCount = Math.max(
    previousProjection?.originalJournals ?? 0,
    ordered.length,
  );
  const originalEventCount = Math.max(
    previousProjection
      ? previousProjection.omittedEvents +
          ordered.reduce((count, journal) => count + journal.events.length, 0)
      : 0,
    retainedOriginalEventCount,
  );
  const build = (
    journals: TodayActivityJournal[],
    textFieldsTruncated: number,
  ) =>
    snapshotSurfaceFromJournals({
      day: surface.day,
      journals,
      maxBytes,
      originalJournalCount,
      originalEventCount,
      textFieldsTruncated,
    });
  const compactedTextCount =
    (previousProjection?.textFieldsTruncated ?? 0) +
    compacted.reduce((count, item) => count + item.textFieldsTruncated, 0);
  let candidate = build(
    compacted.map((item) => item.journal),
    compactedTextCount,
  );
  if (snapshotSurfaceByteLength(candidate) <= maxBytes) return candidate;

  const statusOnly = compacted.map((item) => ({
    ...item.journal,
    events: [],
  }));
  candidate = build(statusOnly, compactedTextCount);
  if (snapshotSurfaceByteLength(candidate) <= maxBytes) return candidate;

  let low = 0;
  let high = statusOnly.length;
  let best = build([], 0);
  if (snapshotSurfaceByteLength(best) > maxBytes) {
    throw new Error("Activity Ledger Today snapshot budget is too small.");
  }
  while (low <= high) {
    const retained = Math.floor((low + high) / 2);
    const retainedJournals = statusOnly.slice(statusOnly.length - retained);
    const retainedTextCount = compacted
      .slice(compacted.length - retained)
      .reduce((count, item) => count + item.textFieldsTruncated, 0);
    const attempt = build(retainedJournals, retainedTextCount);
    if (snapshotSurfaceByteLength(attempt) <= maxBytes) {
      best = attempt;
      low = retained + 1;
    } else {
      high = retained - 1;
    }
  }
  return best;
}

type DecryptObserverEvent = (event: RelayEvent) => Promise<unknown>;
const TODAY_ARCHIVE_DECRYPT_CONCURRENCY = 8;

function localDayForTimestamp(timestamp: string) {
  const date = new Date(timestamp);
  const year = date.getFullYear();
  const month = `${date.getMonth() + 1}`.padStart(2, "0");
  const day = `${date.getDate()}`.padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function journalProjectionKey(agentPubkey: string, journalKey: string) {
  return `${agentPubkey}\u0000${journalKey}`;
}

/**
 * Keep only a bounded, reconstructable working set between archive pages.
 * Unlike the final snapshot projection this never strips every event from a
 * retained journal: older pages may still contain its turn start and
 * correlation root. When the budget is exceeded, the oldest whole journals
 * are omitted and the omission remains explicit in snapshotProjection.
 */
function buildArchiveReconstructionCheckpoint(input: {
  surface: TodayActivitySurface;
  originalJournalCount: number;
  originalEventCount: number;
  previousTextFieldsTruncated: number;
}): BoundedTodayActivitySurface {
  const ordered = [...input.surface.journals].sort(
    (left, right) =>
      Date.parse(left.startedAt) - Date.parse(right.startedAt) ||
      left.id.localeCompare(right.id),
  );
  const compacted = ordered.map((journal) =>
    compactSnapshotJournal(journal, TODAY_ARCHIVE_MAX_EVENTS_PER_JOURNAL),
  );
  const newlyTruncated = compacted.reduce(
    (count, item) => count + item.textFieldsTruncated,
    0,
  );
  const textFieldsTruncated =
    input.previousTextFieldsTruncated + newlyTruncated;
  const build = (journals: TodayActivityJournal[]) =>
    snapshotSurfaceFromJournals({
      day: input.surface.day,
      journals,
      maxBytes: TODAY_SNAPSHOT_SURFACE_MAX_BYTES,
      originalJournalCount: input.originalJournalCount,
      originalEventCount: input.originalEventCount,
      textFieldsTruncated,
    });
  const all = compacted.map((item) => item.journal);
  const candidate = build(all);
  if (
    snapshotSurfaceByteLength(candidate) <= TODAY_SNAPSHOT_SURFACE_MAX_BYTES
  ) {
    return candidate;
  }

  let low = 0;
  let high = all.length;
  let best = build([]);
  while (low <= high) {
    const retained = Math.floor((low + high) / 2);
    const attempt = build(all.slice(all.length - retained));
    if (
      snapshotSurfaceByteLength(attempt) <= TODAY_SNAPSHOT_SURFACE_MAX_BYTES
    ) {
      best = attempt;
      low = retained + 1;
    } else {
      high = retained - 1;
    }
  }
  return best;
}

function observerAgentPubkey(event: RelayEvent): string | null {
  const tag = event.tags.find(
    (candidate) => candidate[0] === "agent" && candidate[1]?.length > 0,
  );
  return tag?.[1] ?? null;
}

function isObserverEvent(value: unknown): value is ObserverEvent {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<ObserverEvent>;
  return (
    Number.isFinite(candidate.seq) &&
    typeof candidate.timestamp === "string" &&
    candidate.timestamp.length > 0 &&
    typeof candidate.kind === "string" &&
    candidate.kind.length > 0 &&
    "payload" in candidate
  );
}

function unwrapObserverEvents(value: unknown): ObserverEvent[] {
  if (!isObserverEvent(value)) return [];
  if (value.kind !== "batch") return [value];
  if (!value.payload || typeof value.payload !== "object") return [];
  const events = (value.payload as { events?: unknown }).events;
  if (!Array.isArray(events)) return [];
  return events.filter(isObserverEvent);
}

/** Return the local-time half-open Unix range used by the owner Today view. */
export function activityLedgerDayRange(day: string): {
  startCreatedAt: number;
  endCreatedAt: number;
} {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
  if (!match) throw new Error("Activity Ledger day must use YYYY-MM-DD.");
  const year = Number(match[1]);
  const monthIndex = Number(match[2]) - 1;
  const date = Number(match[3]);
  const start = new Date(year, monthIndex, date);
  if (
    start.getFullYear() !== year ||
    start.getMonth() !== monthIndex ||
    start.getDate() !== date
  ) {
    throw new Error("Activity Ledger day is not a valid calendar date.");
  }
  const end = new Date(year, monthIndex, date + 1);
  return {
    startCreatedAt: Math.floor(start.getTime() / 1_000),
    endCreatedAt: Math.floor(end.getTime() / 1_000),
  };
}

/**
 * Observer frames are sealed on a one-second publisher tick, so an inner event
 * can cross an outer signed-envelope day boundary. Query a small overlap and
 * let reconstruction assign activity by the authoritative inner timestamp.
 */
export function activityLedgerArchiveQueryRange(day: string): {
  startCreatedAt: number;
  endCreatedAt: number;
} {
  const range = activityLedgerDayRange(day);
  return {
    startCreatedAt:
      range.startCreatedAt - TODAY_ARCHIVE_ENVELOPE_OVERLAP_SECONDS,
    endCreatedAt: range.endCreatedAt + TODAY_ARCHIVE_ENVELOPE_OVERLAP_SECONDS,
  };
}

/**
 * Decrypt and normalize owner-archived observer frames into the Today surface.
 *
 * The signed outer pubkey and `agent` tag must agree with a managed agent. A
 * frame that fails that authority check, fails decryption, or is malformed is
 * excluded instead of being allowed to mint activity or proof.
 */
async function buildTodayActivityFromArchivedEventPages(input: {
  day: string;
  agents: readonly ActivityLedgerAgentIdentity[];
  pages: Iterable<readonly RelayEvent[]> | AsyncIterable<readonly RelayEvent[]>;
  decrypt?: DecryptObserverEvent;
}): Promise<TodayActivitySurface> {
  const decrypt = input.decrypt ?? decryptObserverEvent;
  const trustedAgents = new Map(
    input.agents.map((agent) => [agent.pubkey, agent] as const),
  );
  let retainedEvents = new Map<string, NormalizedActivityEvent[]>();
  const journalEventCounts = new Map<string, number>();
  let originalEventCount = 0;
  let textFieldsTruncated = 0;
  let checkpoint: BoundedTodayActivitySurface | null = null;

  for await (const page of input.pages) {
    const decodedPage: (ObserverEvent[] | null)[] = Array.from(
      { length: page.length },
      () => null,
    );
    let nextIndex = 0;
    const decryptWorker = async () => {
      for (;;) {
        const index = nextIndex;
        nextIndex += 1;
        if (index >= page.length) return;
        const relayEvent = page[index];
        if (!relayEvent) continue;
        const agentPubkey = observerAgentPubkey(relayEvent);
        if (
          !agentPubkey ||
          relayEvent.pubkey !== agentPubkey ||
          !trustedAgents.has(agentPubkey)
        ) {
          continue;
        }

        try {
          const decoded = await decrypt(relayEvent);
          const decodedEvents = unwrapObserverEvents(decoded);
          if (decodedEvents.length === 0) continue;
          decodedPage[index] = decodedEvents;
        } catch {
          // Archive reconciliation is fail-closed: one bad ciphertext cannot
          // suppress the rest of the owner's durable activity surface.
        }
      }
    };
    await Promise.all(
      Array.from(
        { length: Math.min(TODAY_ARCHIVE_DECRYPT_CONCURRENCY, page.length) },
        decryptWorker,
      ),
    );

    // Normalize this page in archive order, not promise-completion order, so
    // reconstruction remains deterministic even when decryption latency
    // varies by frame. Decoded observer bodies live for this page only.
    const pageObserverEvents = new Map<string, ObserverEvent[]>();
    for (let index = 0; index < page.length; index += 1) {
      const relayEvent = page[index];
      const decodedEvents = decodedPage[index];
      if (!relayEvent || !decodedEvents) continue;
      const agentPubkey = observerAgentPubkey(relayEvent);
      if (!agentPubkey) continue;
      const bucket = pageObserverEvents.get(agentPubkey) ?? [];
      for (const decodedEvent of decodedEvents) {
        bucket.push({
          ...decodedEvent,
          sourceEventId: relayEvent.id,
          sourcePubkey: relayEvent.pubkey,
          sourceKind: relayEvent.kind,
          sourceCreatedAt: relayEvent.created_at,
          sourceSignature: relayEvent.sig,
          origin: "historical_backfill",
        });
      }
      pageObserverEvents.set(agentPubkey, bucket);
    }

    const feeds = input.agents.map((agent) => {
      const pageEvents = normalizeActivityEvents(
        pageObserverEvents.get(agent.pubkey) ?? [],
      ).filter((event) => localDayForTimestamp(event.timestamp) === input.day);
      for (const event of pageEvents) {
        originalEventCount += 1;
        const key = journalProjectionKey(agent.pubkey, event.journalKey);
        journalEventCounts.set(key, (journalEventCounts.get(key) ?? 0) + 1);
      }
      return {
        agentPubkey: agent.pubkey,
        agentName: agent.name,
        events: [...(retainedEvents.get(agent.pubkey) ?? []), ...pageEvents],
      };
    });

    const surface = buildTodayActivitySurface(feeds, { day: input.day });
    for (const journal of surface.journals) {
      journal.eventCount =
        journalEventCounts.get(
          journalProjectionKey(journal.agentPubkey, journal.journalKey),
        ) ?? journal.eventCount;
    }
    checkpoint = buildArchiveReconstructionCheckpoint({
      surface,
      originalJournalCount: journalEventCounts.size,
      originalEventCount,
      previousTextFieldsTruncated: textFieldsTruncated,
    });
    textFieldsTruncated = checkpoint.snapshotProjection.textFieldsTruncated;

    // Retain only the bounded normalized projection for the next page. The
    // raw ciphertext page, decoded ObserverEvents, and omitted journal bodies
    // are now unreachable and can be reclaimed before the iterator advances.
    retainedEvents = new Map();
    for (const journal of checkpoint.journals) {
      const bucket = retainedEvents.get(journal.agentPubkey) ?? [];
      bucket.push(...journal.events);
      retainedEvents.set(journal.agentPubkey, bucket);
    }
  }

  return (
    checkpoint ??
    snapshotSurfaceFromJournals({
      day: input.day,
      journals: [],
      maxBytes: TODAY_SNAPSHOT_SURFACE_MAX_BYTES,
      originalJournalCount: 0,
      originalEventCount: 0,
      textFieldsTruncated: 0,
    })
  );
}

export async function buildTodayActivityFromArchivedEvents(input: {
  day: string;
  agents: readonly ActivityLedgerAgentIdentity[];
  events: readonly RelayEvent[];
  decrypt?: DecryptObserverEvent;
}): Promise<TodayActivitySurface> {
  return buildTodayActivityFromArchivedEventPages({
    day: input.day,
    agents: input.agents,
    pages: [input.events],
    decrypt: input.decrypt,
  });
}

/** Reconstruct Today while releasing each raw archive page after decryption. */
export async function buildTodayActivityFromArchivedPages(input: {
  day: string;
  agents: readonly ActivityLedgerAgentIdentity[];
  pages: Iterable<readonly RelayEvent[]> | AsyncIterable<readonly RelayEvent[]>;
  decrypt?: DecryptObserverEvent;
}): Promise<TodayActivitySurface> {
  return buildTodayActivityFromArchivedEventPages(input);
}

/** Apply backend-validated owner artifacts and recompute every derived count. */
export function applyAuthorityToTodayActivity(
  surface: TodayActivitySurface,
  artifacts: readonly ValidatedJournalAuthorityArtifact[],
): TodayActivitySurface {
  const journals: TodayActivityJournal[] = surface.journals.map((journal) => ({
    ...applyValidatedJournalAuthority(journal, artifacts),
    agentPubkey: journal.agentPubkey,
    agentName: journal.agentName,
  }));
  const channels = new Map<
    string,
    {
      journalIds: string[];
      agentPubkeys: Set<string>;
      agentNames: Set<string>;
      lastActivityAt: string;
    }
  >();
  for (const journal of journals) {
    if (!journal.channelId) continue;
    const bucket = channels.get(journal.channelId) ?? {
      journalIds: [],
      agentPubkeys: new Set<string>(),
      agentNames: new Set<string>(),
      lastActivityAt: journal.endedAt,
    };
    bucket.journalIds.push(journal.id);
    bucket.agentPubkeys.add(journal.agentPubkey);
    bucket.agentNames.add(journal.agentName);
    if (Date.parse(journal.endedAt) > Date.parse(bucket.lastActivityAt)) {
      bucket.lastActivityAt = journal.endedAt;
    }
    channels.set(journal.channelId, bucket);
  }

  return {
    ...surface,
    journals,
    channels: [...channels.entries()]
      .map(([channelId, bucket]) => ({
        channelId,
        journalIds: bucket.journalIds,
        agentPubkeys: [...bucket.agentPubkeys],
        agentNames: [...bucket.agentNames],
        lastActivityAt: bucket.lastActivityAt,
      }))
      .sort((left, right) => left.channelId.localeCompare(right.channelId)),
    counts: {
      journals: journals.length,
      failed: journals.filter((journal) => journal.status === "failed").length,
      inProgress: journals.filter((journal) => journal.status === "in_progress")
        .length,
      claimedWithoutEvidence: journals.filter(
        (journal) => journal.claimedCompletionWithoutEvidence,
      ).length,
    },
  };
}
