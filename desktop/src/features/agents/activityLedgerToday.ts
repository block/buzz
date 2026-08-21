import type { RelayEvent } from "@/shared/api/types";
import { decryptObserverEvent } from "@/shared/api/tauriObserver";
import { applyValidatedJournalAuthority } from "./activityLedgerAuthority";
import type { ValidatedJournalAuthorityArtifact } from "./activityLedgerAuthority";
import {
  buildTodayActivitySurface,
  normalizeActivityEvents,
  type TodayActivityJournal,
  type TodayActivitySurface,
} from "./activityLedger";
import type { ObserverEvent } from "./ui/agentSessionTypes";

export type ActivityLedgerAgentIdentity = {
  pubkey: string;
  name: string;
};

type DecryptObserverEvent = (event: RelayEvent) => Promise<unknown>;

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
 * Decrypt and normalize owner-archived observer frames into the Today surface.
 *
 * The signed outer pubkey and `agent` tag must agree with a managed agent. A
 * frame that fails that authority check, fails decryption, or is malformed is
 * excluded instead of being allowed to mint activity or proof.
 */
export async function buildTodayActivityFromArchivedEvents(input: {
  day: string;
  agents: readonly ActivityLedgerAgentIdentity[];
  events: readonly RelayEvent[];
  decrypt?: DecryptObserverEvent;
}): Promise<TodayActivitySurface> {
  const decrypt = input.decrypt ?? decryptObserverEvent;
  const trustedAgents = new Map(
    input.agents.map((agent) => [agent.pubkey, agent] as const),
  );
  const observerEvents = new Map<string, ObserverEvent[]>();

  await Promise.all(
    input.events.map(async (relayEvent) => {
      const agentPubkey = observerAgentPubkey(relayEvent);
      if (
        !agentPubkey ||
        relayEvent.pubkey !== agentPubkey ||
        !trustedAgents.has(agentPubkey)
      ) {
        return;
      }

      try {
        const decoded = await decrypt(relayEvent);
        const decodedEvents = unwrapObserverEvents(decoded);
        if (decodedEvents.length === 0) return;
        const bucket = observerEvents.get(agentPubkey) ?? [];
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
        observerEvents.set(agentPubkey, bucket);
      } catch {
        // Archive reconciliation is fail-closed: one bad ciphertext cannot
        // suppress the rest of the owner's durable activity surface.
      }
    }),
  );

  return buildTodayActivitySurface(
    input.agents.map((agent) => ({
      agentPubkey: agent.pubkey,
      agentName: agent.name,
      events: normalizeActivityEvents(observerEvents.get(agent.pubkey) ?? []),
    })),
    { day: input.day },
  );
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
