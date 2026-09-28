/**
 * Coffee "brewing" UI must track the real in-flight /hula-coffee turn — ACP
 * agent activity on the Assistant DM — not a sticky pendingRun spinner alone.
 *
 * ACP reacts 👀 ("seen"/queued) then 💬 ("working"). Eyes alone are not enough
 * for the Coffee tab; we show 🤔… while the turn is live and disable Brew.
 */

import {
  BESTIE_COFFEE_RUN_MARKER,
  BESTIE_COFFEE_SKILL_COMMAND,
} from "./bestieCoffeeSchedule";
import { BESTIE_JOB_RUN_MARKER } from "./bestieJobSchedule";
import { BESTIE_THREAD_SUMMARIZE_MARKER } from "./bestieThreadProtocol";
import type { BestieCoffeePendingRun } from "./bestieCoffeeTypes";

/** Reminder notify marker — kept local to avoid a WakeController import cycle. */
const BESTIE_REMINDER_NOTIFY_MARKER = "[Bestie reminder]";

/** Thinking-face + ellipsis shown on Coffee tab/sheet while a run is live. */
export const BESTIE_COFFEE_LIVE_LABEL = "🤔…";

/**
 * After Brew / schedule begins, ACP may still only have 👀 while the turn is
 * queued. Keep Brew disabled for this window even before observer "working".
 */
export const BESTIE_COFFEE_START_GRACE_SECONDS = 15;

/**
 * After start grace, if ACP is idle for this long, treat pending as stale.
 * Keeps a short settle window so a reply can land after working→idle.
 */
export const BESTIE_COFFEE_IDLE_SETTLE_SECONDS = 45;

/**
 * Hard cap: clear pending even if the working signal is stuck, so Brew/👀
 * cannot hang for an hour.
 */
export const BESTIE_COFFEE_HARD_TIMEOUT_SECONDS = 8 * 60;

/** @deprecated Use IDLE_SETTLE + HARD_TIMEOUT; kept for older test imports. */
export const BESTIE_COFFEE_STALE_PENDING_SECONDS =
  BESTIE_COFFEE_START_GRACE_SECONDS + BESTIE_COFFEE_IDLE_SETTLE_SECONDS;

/** Failure text when a brew/schedule never gets a matching Assistant reply. */
export const BESTIE_COFFEE_ABANDONED_OUTPUT =
  "Brew timed out — no Assistant reply was captured for this /hula-coffee run.";

export function messageLooksLikeBestieCoffeeTrigger(content: string): boolean {
  if (!content) return false;
  if (content.includes(BESTIE_COFFEE_RUN_MARKER)) return true;
  if (content.includes(BESTIE_COFFEE_SKILL_COMMAND)) return true;
  return /(^|\n)\s*\/hula-coffee(\s|$)/m.test(content);
}

/**
 * Owner turns that are *not* coffee but still wake ACP on the Assistant DM
 * (Thread Summarize, Jobs, Reminder notify). While one of these is the latest
 * open system turn, 🤔… / Brewing must not claim a coffee run.
 */
export function messageLooksLikeBestieCompetingSystemTrigger(
  content: string,
): boolean {
  if (!content) return false;
  if (content.includes(BESTIE_THREAD_SUMMARIZE_MARKER)) return true;
  if (content.includes(BESTIE_JOB_RUN_MARKER)) return true;
  if (content.includes(BESTIE_REMINDER_NOTIFY_MARKER)) return true;
  return false;
}

/**
 * ACP "seen" / "working" reactions are kind:7 with content 👀 / 💬.
 * They share e-tags with the coffee trigger but must NEVER finalize a Coffee
 * row — otherwise Path A/D captures 👀 and the real kind:9 brief cannot upgrade.
 */
const BESTIE_ACP_STATUS_EMOJI = new Set(["👀", "💬"]);

export function isBestieAcpStatusReactionContent(
  content: string | null | undefined,
): boolean {
  if (typeof content !== "string") return false;
  const trimmed = content.trim();
  return BESTIE_ACP_STATUS_EMOJI.has(trimmed);
}

/** True when an event kind is a chat message row (not reaction/aux). */
export function isBestieCoffeeCaptureMessageKind(kind: number | null | undefined): boolean {
  return kind === 9 || kind === 40002;
}

/** NIP-10 reply parent from kind:9 tags (explicit reply marker, else last e). */
export function replyParentIdFromEventTags(
  tags: readonly (readonly string[])[] | null | undefined,
): string | null {
  if (!tags || tags.length === 0) return null;
  let reply: string | null = null;
  let lastE: string | null = null;
  for (const tag of tags) {
    if (!tag || tag[0] !== "e" || !tag[1]) continue;
    // Lowercase — harness / CLI e-tags may differ in hex case from stored ids.
    const id = tag[1].toLowerCase();
    lastE = id;
    if (tag[3] === "reply") reply = id;
  }
  return reply ?? lastE;
}

/**
 * True while /hula-coffee is actually in flight for Brew / schedule UI.
 *
 * - Prefer ACP agent-working on the Assistant DM once a coffee run is open.
 * - During start grace, stay live even if only 👀 has landed (not working yet).
 * - Optional `openCoffeeTriggerAt`: latest unmatched coffee user-turn time so a
 *   manual `/hula-coffee` (no pendingRun yet) still disables Brew while ACP works.
 */
export function isBestieCoffeeLive(options: {
  agentWorkingOnBestieDm: boolean;
  nowSeconds: number;
  openCoffeeTriggerAt?: number | null;
  /**
   * Latest owner system turn that is *not* coffee (summarize / job / reminder
   * notify). When newer than the coffee trigger / pending start, ACP working
   * belongs to that turn — do not show 🤔… as brewing.
   */
  latestCompetingTriggerAt?: number | null;
  pendingRun: BestieCoffeePendingRun | null;
}): boolean {
  const { agentWorkingOnBestieDm, nowSeconds, pendingRun } = options;
  const openTriggerAt = options.openCoffeeTriggerAt ?? null;
  const competingAt = options.latestCompetingTriggerAt ?? null;

  const coffeeStillLatestOpen = (coffeeAt: number): boolean =>
    competingAt == null || coffeeAt >= competingAt;

  if (pendingRun) {
    const age = nowSeconds - pendingRun.startedAt;
    // Queued 👀 window after Brew/schedule — coffee owns this even before ACP working.
    if (age >= 0 && age <= BESTIE_COFFEE_START_GRACE_SECONDS) {
      // If a newer summarize/job already started, grace no longer applies.
      if (!coffeeStillLatestOpen(pendingRun.startedAt)) return false;
      return true;
    }
    // ACP working only counts as coffee when coffee is still the latest system turn.
    if (agentWorkingOnBestieDm && coffeeStillLatestOpen(pendingRun.startedAt)) {
      return true;
    }
    return false;
  }

  // Manual /hula-coffee without local lock: only while ACP works *and* coffee
  // is still the latest open system trigger (not Thread Summarize, etc.).
  if (
    agentWorkingOnBestieDm &&
    openTriggerAt != null &&
    openTriggerAt <= nowSeconds &&
    coffeeStillLatestOpen(openTriggerAt)
  ) {
    return true;
  }

  return false;
}

/**
 * Pending lock that should be cleared / finalized:
 * - hard timeout (even if working signal stuck)
 * - ACP idle past grace + settle
 * - a newer competing system turn (summarize/job/reminder) after grace
 */
export function isBestieCoffeePendingStale(options: {
  agentWorkingOnBestieDm: boolean;
  nowSeconds: number;
  pendingRun: BestieCoffeePendingRun | null;
  latestCompetingTriggerAt?: number | null;
}): boolean {
  const pending = options.pendingRun;
  if (!pending) return false;
  const age = options.nowSeconds - pending.startedAt;
  if (age < 0) return false;
  if (age >= BESTIE_COFFEE_HARD_TIMEOUT_SECONDS) return true;

  const competingAt = options.latestCompetingTriggerAt ?? null;
  // Superseded by a newer system turn: only abandon once ACP is idle so we
  // do not finalize while the original reply may still be in flight.
  if (
    competingAt != null &&
    competingAt > pending.startedAt &&
    age > BESTIE_COFFEE_START_GRACE_SECONDS &&
    !options.agentWorkingOnBestieDm
  ) {
    return true;
  }

  if (options.agentWorkingOnBestieDm) return false;
  if (age <= BESTIE_COFFEE_START_GRACE_SECONDS) return false;
  return age >= BESTIE_COFFEE_START_GRACE_SECONDS + BESTIE_COFFEE_IDLE_SETTLE_SECONDS;
}

/**
 * Disable Brew when a coffee turn is live, or while we still hold a fresh
 * pending lock (grace window / about to send).
 */
export function shouldDisableBestieCoffeeBrew(options: {
  agentWorkingOnBestieDm: boolean;
  nowSeconds: number;
  openCoffeeTriggerAt?: number | null;
  latestCompetingTriggerAt?: number | null;
  pendingRun: BestieCoffeePendingRun | null;
}): boolean {
  if (
    isBestieCoffeeLive({
      agentWorkingOnBestieDm: options.agentWorkingOnBestieDm,
      nowSeconds: options.nowSeconds,
      openCoffeeTriggerAt: options.openCoffeeTriggerAt,
      latestCompetingTriggerAt: options.latestCompetingTriggerAt,
      pendingRun: options.pendingRun,
    })
  ) {
    return true;
  }
  // Fresh pending before live grace evaluation edge — still block double Brew.
  const pending = options.pendingRun;
  if (!pending) return false;
  const competingAt = options.latestCompetingTriggerAt ?? null;
  if (competingAt != null && competingAt > pending.startedAt) return false;
  const age = options.nowSeconds - pending.startedAt;
  return age >= 0 && age <= BESTIE_COFFEE_START_GRACE_SECONDS;
}
