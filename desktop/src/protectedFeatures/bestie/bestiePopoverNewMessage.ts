/**
 * Assistant popover — detect new messages the user is not currently looking at.
 */

import { BESTIE_COFFEE_RUN_MARKER } from "./bestieCoffeeSchedule";
import { BESTIE_JOB_RUN_MARKER } from "./bestieJobSchedule";
import { BESTIE_LIVE_LIST_STATE_TURN_HINT_MARKER } from "./bestieOutboundHints";
import { BESTIE_LIST_TURN_HINT_MARKER } from "./bestieListProtocol";
import { BESTIE_JOB_TURN_HINT_MARKER } from "./bestieJobProtocol";
import { BESTIE_THREAD_SUMMARIZE_MARKER } from "./bestieThreadProtocol";

export type BestiePopoverNewMessageTarget = {
  /** Message id to jump to / dismiss. */
  id: string;
  /** True when the message is outside the active session filter. */
  outsideSession: boolean;
  /** Truncated plain preview of the message body. */
  preview: string;
  /** Thread root id used to queue one banner per thread. */
  threadRootId: string;
};

const DEFAULT_PREVIEW_MAX = 140;

/** Top-level system / injected Bestie turns — never New-message noise. */
const BESTIE_SYSTEM_NOISE_MARKERS = [
  BESTIE_COFFEE_RUN_MARKER,
  BESTIE_JOB_RUN_MARKER,
  "[Bestie reminder]",
  BESTIE_THREAD_SUMMARIZE_MARKER,
  BESTIE_LIST_TURN_HINT_MARKER,
  BESTIE_JOB_TURN_HINT_MARKER,
  BESTIE_LIVE_LIST_STATE_TURN_HINT_MARKER,
] as const;

/**
 * True for injected Bestie system turns (coffee / jobs / reminder / summarize /
 * teach hints) that must not drive the New message banner.
 */
export function isBestiePopoverSystemNoise(
  body: string | null | undefined,
): boolean {
  if (typeof body !== "string" || body.length === 0) return false;
  for (const marker of BESTIE_SYSTEM_NOISE_MARKERS) {
    if (body.includes(marker)) return true;
  }
  // Bare skill invoke without the coffee marker (manual /hula-coffee).
  if (/(^|\n)\s*\/hula-coffee(\s|$)/m.test(body)) return true;
  return false;
}

/** Collapse whitespace and truncate for the New message banner body. */
export function previewBestiePopoverMessageBody(
  body: string | null | undefined,
  max = DEFAULT_PREVIEW_MAX,
): string {
  const compact = (body ?? "").replace(/\s+/g, " ").trim();
  if (!compact) return "";
  if (compact.length <= max) return compact;
  return `${compact.slice(0, Math.max(1, max - 1)).trimEnd()}…`;
}

function threadRootIdFor(message: {
  id: string;
  parentId?: string | null;
  rootId?: string | null;
}): string {
  return message.rootId ?? message.parentId ?? message.id;
}

function toTarget(
  message: {
    id: string;
    body?: string | null;
    parentId?: string | null;
    rootId?: string | null;
  },
  outsideSession: boolean,
): BestiePopoverNewMessageTarget {
  return {
    id: message.id,
    outsideSession,
    preview: previewBestiePopoverMessageBody(message.body),
    threadRootId: threadRootIdFor(message),
  };
}

function isSkipped(
  message: {
    id: string;
    createdAt: number;
    body?: string | null;
    parentId?: string | null;
    rootId?: string | null;
  },
  input: {
    ignoreMessageId?: string | null;
    dismissedMessageIds?: ReadonlySet<string>;
    dismissedThreadRootIds?: ReadonlySet<string>;
    baselineMessageIds?: ReadonlySet<string>;
    minCreatedAt?: number | null;
  },
): boolean {
  const ignore = input.ignoreMessageId?.trim() || null;
  if (ignore && message.id === ignore) return true;
  if (input.baselineMessageIds?.has(message.id)) return true;
  if (input.dismissedMessageIds?.has(message.id)) return true;
  const rootId = threadRootIdFor(message);
  if (input.dismissedThreadRootIds?.has(rootId)) return true;
  if (
    input.minCreatedAt != null &&
    Number.isFinite(input.minCreatedAt) &&
    message.createdAt < input.minCreatedAt
  ) {
    return true;
  }
  if (isBestiePopoverSystemNoise(message.body)) return true;
  return false;
}

/**
 * Build the New message banner queue (newest first).
 *
 * - Outside-session messages: one entry per thread (newest message in that
 *   thread). View opens the Assistant DM on that thread; popover session stays.
 * - Otherwise, when scrolled away from the bottom: a single in-session entry
 *   (View scrolls within the popover transcript).
 * - Dismissed message / thread ids stay out permanently.
 * - Baseline (already-in-view when the session started) and Bestie system
 *   noise never queue.
 * - With no active session, historical DMs are not treated as "new".
 */
export function resolveBestiePopoverNewMessageQueue(input: {
  /** Newest-first or any order; we pick by createdAt then id. */
  allMessages: readonly {
    id: string;
    createdAt: number;
    body?: string | null;
    parentId?: string | null;
    rootId?: string | null;
  }[];
  sessionMessageIds: ReadonlySet<string>;
  sessionRootId?: string | null;
  /** User is scrolled near the transcript bottom. */
  nearBottom: boolean;
  /** Ignore the user's own outbound id so sending doesn't flash the banner. */
  ignoreMessageId?: string | null;
  /** Message ids the user dismissed / viewed from the banner queue. */
  dismissedMessageIds?: ReadonlySet<string>;
  /** Thread roots dismissed / viewed — stay clear until a newer msg arrives. */
  dismissedThreadRootIds?: ReadonlySet<string>;
  /** Ids present before the active session (already in view). */
  baselineMessageIds?: ReadonlySet<string>;
  /** Ignore messages strictly older than this (session start). */
  minCreatedAt?: number | null;
}): BestiePopoverNewMessageTarget[] {
  const hasSession =
    Boolean(input.sessionRootId) || input.sessionMessageIds.size > 0;

  const sorted = [...input.allMessages].sort((a, b) => {
    if (a.createdAt !== b.createdAt) return b.createdAt - a.createdAt;
    return b.id.localeCompare(a.id);
  });

  // No active session: do not flood with the entire DM history as "new".
  if (!hasSession) {
    return [];
  }

  const seenThreads = new Set<string>();
  const outsideQueue: BestiePopoverNewMessageTarget[] = [];
  for (const message of sorted) {
    if (input.sessionMessageIds.has(message.id)) continue;
    const rootId = threadRootIdFor(message);
    // Newest message claims the thread slot first. If that newest id / root
    // was dismissed (or is noise/baseline), skip the whole thread until a
    // newer message arrives — do not fall back to older siblings.
    if (seenThreads.has(rootId)) continue;
    seenThreads.add(rootId);
    if (isSkipped(message, input)) continue;
    outsideQueue.push(toTarget(message, true));
  }
  if (outsideQueue.length > 0) return outsideQueue;

  if (input.nearBottom) return [];
  const newestInSession = sorted.find((message) => {
    if (!input.sessionMessageIds.has(message.id)) return false;
    if (isSkipped(message, input)) return false;
    return true;
  });
  if (!newestInSession) return [];
  return [toTarget(newestInSession, false)];
}

/**
 * Pick the front of the New message queue (or null when empty).
 */
export function resolveBestiePopoverNewMessageTarget(input: {
  allMessages: readonly {
    id: string;
    createdAt: number;
    body?: string | null;
    parentId?: string | null;
    rootId?: string | null;
  }[];
  sessionMessageIds: ReadonlySet<string>;
  sessionRootId?: string | null;
  nearBottom: boolean;
  ignoreMessageId?: string | null;
  dismissedMessageIds?: ReadonlySet<string>;
  dismissedThreadRootIds?: ReadonlySet<string>;
  baselineMessageIds?: ReadonlySet<string>;
  minCreatedAt?: number | null;
}): BestiePopoverNewMessageTarget | null {
  return resolveBestiePopoverNewMessageQueue(input)[0] ?? null;
}

/**
 * Ids to permanently ack when the user Dismisses / Views a banner target
 * (message id + its thread root).
 */
export function bestiePopoverNewMessageAckIds(
  target: BestiePopoverNewMessageTarget,
): { messageId: string; threadRootId: string } {
  return { messageId: target.id, threadRootId: target.threadRootId };
}
