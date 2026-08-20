import { normalizePubkey } from "@/shared/lib/pubkey";
import type { ActiveChannelTurnSummary } from "@/features/agents/activeAgentTurnsStore";
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
  source: "manual" | "derived";
};

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
    waits.push({ ...value, actor: { ...value.actor }, source: "manual" });
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
