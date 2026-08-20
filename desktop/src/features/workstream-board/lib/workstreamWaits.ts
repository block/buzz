import type { ActiveChannelTurnSummary } from "@/features/agents/activeAgentTurnsStore";
import { parseMessageLink } from "@/features/messages/lib/messageLink";
import type { ParsedMessageLink } from "@/features/messages/lib/messageLink";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { WorkstreamCardV1 } from "@/features/workstream-board/lib/workstreamCardParser";
import type {
  WorkstreamPullRequestDisplayState,
  WorkstreamPullRequestReference,
} from "@/features/workstream-board/lib/workstreamPullRequestStatus";

export type WorkstreamWaitActor = {
  kind: "person" | "agent" | "team";
  name: string;
  pubkey?: string;
};

export type WorkstreamWait = {
  key: string;
  actor: WorkstreamWaitActor;
  reason: string;
  since?: string;
  /** Optional canonical `buzz://message?...` destination from the canvas. */
  message?: string;
  /** True when the canvas explicitly supplied a `message` field, even if malformed. */
  messagePresent?: boolean;
  source: "manual" | "derived";
};

const CHANNEL_ID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const EVENT_ID_PATTERN = /^[0-9a-f]{64}$/;

/**
 * Parse and strictly validate a manual Workstream Board message destination.
 *
 * `parseMessageLink` intentionally accepts any non-empty route values for
 * general markdown links. Canvas-authored blockers need the stronger wire
 * contract: a canonical URL, a UUID channel ID, and 64-hex event IDs.
 */
export function parseWorkstreamWaitMessageLink(
  value: unknown,
): ParsedMessageLink | null {
  if (typeof value !== "string") return null;
  const parsed = parseMessageLink(value);
  if (!parsed.ok) return null;
  const { channelId, messageId, threadRootId } = parsed.value;
  if (
    !CHANNEL_ID_PATTERN.test(channelId) ||
    !EVENT_ID_PATTERN.test(messageId) ||
    (threadRootId !== null && !EVENT_ID_PATTERN.test(threadRootId))
  ) {
    return null;
  }

  // Reject encoded values, unknown query parameters, and non-canonical query
  // ordering rather than silently navigating to a different destination.
  const canonical = `buzz://message?channel=${channelId}&id=${messageId}${
    threadRootId ? `&thread=${threadRootId}` : ""
  }`;
  return canonical === value ? parsed.value : null;
}

export type WorkstreamCardSortInput = {
  channelId: string;
  channelName: string;
  createdAt: string | null | undefined;
  waits: readonly WorkstreamWait[];
  activeWorking?: ActiveChannelTurnSummary;
};

export function isWorkstreamWait(
  value: unknown,
): value is Omit<WorkstreamWait, "source"> {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    return false;
  const wait = value as Record<string, unknown>;
  if (
    typeof wait.key !== "string" ||
    !wait.key.trim() ||
    typeof wait.reason !== "string" ||
    !wait.reason.trim()
  )
    return false;
  if (
    typeof wait.actor !== "object" ||
    wait.actor === null ||
    Array.isArray(wait.actor)
  )
    return false;
  const actor = wait.actor as Record<string, unknown>;
  if (
    actor.kind !== "person" &&
    actor.kind !== "agent" &&
    actor.kind !== "team"
  )
    return false;
  if (typeof actor.name !== "string" || !actor.name.trim()) return false;
  if (
    wait.since !== undefined &&
    (typeof wait.since !== "string" || !Number.isFinite(Date.parse(wait.since)))
  )
    return false;
  // Keep malformed destinations attached to an otherwise valid blocker. The
  // renderer validates the link before making the row interactive.
  return (
    actor.pubkey === undefined ||
    (typeof actor.pubkey === "string" && !!actor.pubkey.trim())
  );
}

export function parseManualWorkstreamWaits(
  values: readonly unknown[],
): WorkstreamWait[] {
  const waits: WorkstreamWait[] = [];
  const keys = new Set<string>();
  for (const value of values) {
    if (!isWorkstreamWait(value) || keys.has(value.key)) continue;
    keys.add(value.key);
    const messagePresent = Object.hasOwn(value, "message");
    const message =
      typeof value.message === "string" ? value.message : undefined;
    waits.push({
      key: value.key,
      actor: { ...value.actor },
      reason: value.reason,
      ...(value.since === undefined ? {} : { since: value.since }),
      ...(messagePresent ? { message, messagePresent: true } : {}),
      source: "manual",
    });
  }
  return waits;
}

function needsReviewerWait(
  state: WorkstreamPullRequestDisplayState | undefined,
): state is Extract<WorkstreamPullRequestDisplayState, { status: "live" }> {
  return (
    state?.status === "live" &&
    state.lifecycle === "open" &&
    state.reviewState !== "approved"
  );
}

export function deriveReviewerWaits(
  references: readonly WorkstreamPullRequestReference[],
  states: ReadonlyMap<string, WorkstreamPullRequestDisplayState>,
): WorkstreamWait[] {
  return references.flatMap((reference) => {
    const state = states.get(reference.identity);
    if (!needsReviewerWait(state)) return [];
    return [
      {
        key: reference.identity,
        actor: { kind: "team", name: "Reviewers" },
        reason: `Review requested for ${state.repository}${state.number ? ` #${state.number}` : ""}`,
        source: "derived" as const,
      },
    ];
  });
}

/** Manual wording wins for an intentionally matching derived PR key. */
export function mergeWorkstreamWaits(
  manual: readonly WorkstreamWait[],
  derived: readonly WorkstreamWait[],
): WorkstreamWait[] {
  const merged = new Map<string, WorkstreamWait>();
  for (const wait of derived) merged.set(wait.key, wait);
  for (const wait of manual) merged.set(wait.key, wait);
  return [...merged.values()];
}

function isCurrentOwnerWait(
  wait: WorkstreamWait,
  currentOwnerPubkey?: string | null,
): boolean {
  return (
    !!currentOwnerPubkey &&
    !!wait.actor.pubkey &&
    normalizePubkey(wait.actor.pubkey) === normalizePubkey(currentOwnerPubkey)
  );
}

function createdAtMillis(value: string | null | undefined): number {
  const parsed = value ? Date.parse(value) : Number.NaN;
  return Number.isFinite(parsed) ? parsed : Number.MAX_SAFE_INTEGER;
}

/** Stable board priority: current owner wait, active work, creation time, then name and ID. */
export function sortWorkstreamCards<T extends WorkstreamCardSortInput>(
  cards: readonly T[],
  currentOwnerPubkey?: string | null,
): T[] {
  return [...cards].sort((left, right) => {
    const leftOwner = left.waits.some((wait) =>
      isCurrentOwnerWait(wait, currentOwnerPubkey),
    );
    const rightOwner = right.waits.some((wait) =>
      isCurrentOwnerWait(wait, currentOwnerPubkey),
    );
    if (leftOwner !== rightOwner) return leftOwner ? -1 : 1;
    const leftActive = Boolean(left.activeWorking);
    const rightActive = Boolean(right.activeWorking);
    if (leftActive !== rightActive) return leftActive ? -1 : 1;
    const creationDelta =
      createdAtMillis(left.createdAt) - createdAtMillis(right.createdAt);
    if (creationDelta !== 0) return creationDelta;
    const nameDelta = left.channelName.localeCompare(right.channelName);
    return nameDelta !== 0
      ? nameDelta
      : left.channelId.localeCompare(right.channelId);
  });
}

export function waitsForCard(
  card: WorkstreamCardV1,
  references: readonly WorkstreamPullRequestReference[],
  states: ReadonlyMap<string, WorkstreamPullRequestDisplayState>,
): WorkstreamWait[] {
  return mergeWorkstreamWaits(
    parseManualWorkstreamWaits(card.waitingOn),
    deriveReviewerWaits(references, states),
  );
}
