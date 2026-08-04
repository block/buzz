import type { InboxItem } from "@/features/home/lib/inbox";
import { getThreadReference } from "@/features/messages/lib/threading";
import {
  type DeclaredAsk,
  parseDeclaredAsks,
  stripDeclaredAskLines,
  viewerAsks,
} from "@/features/attention/lib/declaredAsks";
import {
  type AskType,
  classifyAsk,
  MAX_ASK_COUNT,
} from "@/features/attention/lib/taskExtraction";

export type { DeclaredAsk } from "@/features/attention/lib/declaredAsks";
export type { AskType } from "@/features/attention/lib/taskExtraction";

export type AttentionZone = "needsMe" | "waiting" | "done";

export type ZoneStateEntry = {
  zone: "waiting" | "done";
  /** Unix seconds when the user parked the item in this zone. */
  changedAt: number;
  /**
   * Unix seconds when the parking action's reply actually published
   * (set at commit, absent for silent actions). Re-parking clears it.
   */
  respondedAt?: number;
};

/** Keyed by the inbox item's stable conversation id. */
export type ZoneStateMap = Record<string, ZoneStateEntry>;

export type AttentionItem = {
  /** Stable identity — the inbox conversation id. */
  id: string;
  inboxItem: InboxItem;
  /** Why this needs the user, in one short phrase. */
  reason: string;
  /** The one-line ask headline; null for headsUp items. */
  ask: string | null;
  askType: AskType;
  /** Distinct qualifying asks in the message; >1 renders a multi-ask headline. */
  askCount: number;
  /**
   * Tier-1 declared asks addressed to the viewer (base_prompt.md "Declare
   * what you need" convention). Present only when the declared tier won.
   */
  declaredAsks?: DeclaredAsk[];
  zone: AttentionZone;
  zoneChangedAt: number | null;
  /**
   * True when the item was parked in Waiting or Done but new activity arrived
   * afterwards, pulling it back into Needs Me.
   */
  reactivated: boolean;
  /**
   * True on a reactivated card whose parking action already published its
   * reply. Without this flag the returning card reads as the action having
   * failed, and the user answers twice.
   */
  responded: boolean;
};

export type AttentionProjection = {
  needsMe: AttentionItem[];
  /** Mentions with no detectable ask — demoted, never counted as Needs Me. */
  headsUp: AttentionItem[];
  waiting: AttentionItem[];
  done: AttentionItem[];
};

export const DONE_RETENTION_SECONDS = 7 * 24 * 60 * 60;
export const MAX_ZONE_ENTRIES = 500;

const KIND_WORKFLOW_APPROVAL_REQUESTED = 46010;
const KIND_STREAM_REMINDER = 40007;

/**
 * Attention only surfaces items that plausibly need the user: action-required
 * feed items and direct mentions. Ambient channel activity stays in the
 * Inbox — the two surfaces answer different questions.
 */
export function isAttentionWorthy(item: InboxItem): boolean {
  return (
    item.isActionRequired ||
    item.categories.includes("mention") ||
    item.categories.includes("needs_action")
  );
}

export function attentionReason(item: InboxItem): string {
  if (item.categories.includes("needs_action")) {
    if (item.item.kind === KIND_WORKFLOW_APPROVAL_REQUESTED) {
      return "Approval requested";
    }
    if (item.item.kind === KIND_STREAM_REMINDER) {
      return "Reminder due";
    }
    return "Action required";
  }
  if (item.categories.includes("mention")) {
    return item.groupItems.length > 1
      ? "Mentioned you in an active thread"
      : "Mentioned you";
  }
  if (item.categories.includes("agent_activity")) {
    return "Agent update";
  }
  return "New activity";
}

/** NIP-10 thread root of the representative event, for deep links. */
export function attentionThreadRootId(item: InboxItem): string | null {
  return getThreadReference(item.item.tags).rootId;
}

function classifyInboxItem(
  item: InboxItem,
  viewerName: string | undefined,
): {
  ask: string | null;
  askType: AskType;
  askCount: number;
  declaredAsks?: DeclaredAsk[];
} {
  // Tier 1: declared asks always beat the derived heuristics — the author
  // stated exactly who they need and what for.
  const declared = parseDeclaredAsks(item.item.content);
  const mine = viewerAsks(declared, viewerName);
  if (mine.length > 0) {
    return {
      ask: mine[0].ask,
      askType: mine[0].type,
      askCount: Math.min(mine.length, MAX_ASK_COUNT),
      declaredAsks: mine,
    };
  }

  // Tier 2: derived classification. Declarations addressed to other people
  // are stripped first so a message that only needs someone else stays
  // headsUp unless the remaining prose carries its own ask.
  const derivedSource =
    declared.length > 0
      ? stripDeclaredAskLines(item.item.content)
      : item.item.content;
  const classification = classifyAsk(derivedSource);
  // Kind-level needs_action events carry their type; prose is classified.
  if (item.item.kind === KIND_WORKFLOW_APPROVAL_REQUESTED) {
    return {
      ask: classification.ask ?? item.preview,
      askType: "approval",
      askCount: Math.max(1, classification.askCount),
    };
  }
  if (item.item.kind === KIND_STREAM_REMINDER) {
    return {
      ask: classification.ask ?? item.preview,
      askType:
        classification.type === "headsUp" ? "review" : classification.type,
      askCount: Math.max(1, classification.askCount),
    };
  }
  return {
    ask: classification.ask,
    askType: classification.type,
    askCount: classification.askCount,
  };
}

function toAttentionItem(
  item: InboxItem,
  zone: AttentionZone,
  entry: ZoneStateEntry | undefined,
  reactivated: boolean,
  options: ProjectAttentionOptions | undefined,
): AttentionItem {
  const { ask, askType, askCount, declaredAsks } = classifyInboxItem(
    item,
    options?.viewerName,
  );
  // A local badge correction wins over both tiers; it never reclassifies
  // the declared sub-asks, only the card's presented type (and bucket).
  const override = options?.badgeOverrides?.[item.conversationId];
  return {
    id: item.conversationId,
    inboxItem: item,
    reason: attentionReason(item),
    ask,
    askType: override ?? askType,
    askCount,
    declaredAsks,
    zone,
    zoneChangedAt: entry?.changedAt ?? null,
    reactivated,
    responded: reactivated && entry?.respondedAt != null,
  };
}

export type AttentionAction = "done" | "noted" | "waiting" | "handledElsewhere";

/**
 * The frozen Handled-elsewhere line (UX pass, "The wording"). Honest on
 * all three readings of "handled": it stops the waiting, does not fake a
 * resolution in the ask's favour, and gives the agent a defined path if
 * it genuinely still needs the answer.
 */
export const HANDLED_ELSEWHERE_REPLY =
  "Handled outside this thread, no answer coming here. If you are still blocked, say so and I will pick it up.";

/**
 * An action posts a reply when someone is waiting for it. Nobody waits on
 * a To note item, so Noted there is local-only — nothing publishes. The
 * check uses the item's CURRENT effective type, so a badge-corrected card
 * follows its corrected bucket.
 */
export function actionPublishes(
  askType: AskType,
  action: AttentionAction,
): boolean {
  return !(action === "noted" && askType === "headsUp");
}

/**
 * Locked action matrix: the generic action a card presents as primary.
 * Done only where it genuinely means "I performed the manual step you
 * needed" (Blocked); Noted only where nothing publishes (Heads up). The
 * option-driven types (approval, decision, question, review) have option
 * primaries instead and return null here.
 */
export function primaryGenericAction(
  askType: AskType,
): "done" | "noted" | null {
  if (askType === "blocked") return "done";
  if (askType === "headsUp") return "noted";
  return null;
}

/**
 * Handled elsewhere exists for every actionable type except Blocked
 * (where Done already is the correct signal) and Heads up (where nothing
 * publishes at all).
 */
export function offersHandledElsewhere(askType: AskType): boolean {
  return askType !== "blocked" && askType !== "headsUp";
}

/** Whole days an item has been sitting on the user. 0 = under a day. */
export function waitingDays(
  latestActivityAt: number,
  nowSeconds: number,
): number {
  return Math.max(0, Math.floor((nowSeconds - latestActivityAt) / 86_400));
}

/** Local-day bucket used by the Needs Me headers. */
export function isSameLocalDay(aSeconds: number, bSeconds: number): boolean {
  return (
    new Date(aSeconds * 1_000).toDateString() ===
    new Date(bSeconds * 1_000).toDateString()
  );
}

export type ProjectAttentionOptions = {
  /** Viewer display name for the declared-ask tier ("Needs <Name>, …"). */
  viewerName?: string;
  /** Local badge corrections, keyed by conversation id. */
  badgeOverrides?: Record<string, AskType>;
};

/**
 * Split attention-worthy inbox items into the three Attention views.
 *
 * Rules:
 * - No zone entry → Needs Me.
 * - Activity newer than the zone entry reactivates the item into Needs Me,
 *   whether it was Waiting or Done — the underlying conversation moved, so
 *   the user's park decision is stale.
 * - Waiting and Done otherwise honour the stored zone.
 * - Done items expire from view after DONE_RETENTION_SECONDS.
 */
export function projectAttention(
  items: InboxItem[],
  zoneState: ZoneStateMap,
  nowSeconds: number,
  options?: ProjectAttentionOptions,
): AttentionProjection {
  const needsMe: AttentionItem[] = [];
  const headsUp: AttentionItem[] = [];
  const waiting: AttentionItem[] = [];
  const done: AttentionItem[] = [];

  for (const item of items) {
    if (!isAttentionWorthy(item)) {
      continue;
    }
    const entry = zoneState[item.conversationId];
    let projected: AttentionItem;
    if (!entry) {
      projected = toAttentionItem(item, "needsMe", undefined, false, options);
    } else if (item.latestActivityAt > entry.changedAt) {
      projected = toAttentionItem(item, "needsMe", entry, true, options);
    } else if (entry.zone === "waiting") {
      waiting.push(toAttentionItem(item, "waiting", entry, false, options));
      continue;
    } else {
      if (nowSeconds - entry.changedAt <= DONE_RETENTION_SECONDS) {
        done.push(toAttentionItem(item, "done", entry, false, options));
      }
      continue;
    }
    // Demotion tier: no detectable ask means it is not Needs Me.
    if (projected.askType === "headsUp") {
      headsUp.push(projected);
    } else {
      needsMe.push(projected);
    }
  }

  // Staleness is the cost: the oldest open ask sorts to the top.
  needsMe.sort(
    (a, b) => a.inboxItem.latestActivityAt - b.inboxItem.latestActivityAt,
  );
  headsUp.sort(
    (a, b) => b.inboxItem.latestActivityAt - a.inboxItem.latestActivityAt,
  );
  waiting.sort((a, b) => (b.zoneChangedAt ?? 0) - (a.zoneChangedAt ?? 0));
  done.sort((a, b) => (b.zoneChangedAt ?? 0) - (a.zoneChangedAt ?? 0));

  return { needsMe, headsUp, waiting, done };
}

/**
 * Zone state that is safe to persist. Entries still inside an undo hold
 * window are excluded: a held action that survives an app quit would leave
 * the card cleared while its reply never publishes — the exact split state
 * the undo guarantee forbids. Held entries persist only on commit.
 */
export function persistableZoneState(
  state: ZoneStateMap,
  holdIds: ReadonlySet<string>,
): ZoneStateMap {
  if (holdIds.size === 0) return state;
  return Object.fromEntries(
    Object.entries(state).filter(([id]) => !holdIds.has(id)),
  );
}

/**
 * Drop expired Done entries and cap the map so localStorage stays bounded.
 * Oldest entries fall out first when over the cap.
 */
export function pruneZoneState(
  state: ZoneStateMap,
  nowSeconds: number,
  maxEntries: number = MAX_ZONE_ENTRIES,
): ZoneStateMap {
  const entries = Object.entries(state).filter(
    ([, entry]) =>
      entry.zone !== "done" ||
      nowSeconds - entry.changedAt <= DONE_RETENTION_SECONDS,
  );
  entries.sort(([, a], [, b]) => b.changedAt - a.changedAt);
  return Object.fromEntries(entries.slice(0, maxEntries));
}
