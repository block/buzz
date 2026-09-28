import * as React from "react";

/**
 * Footer-avatar attention for Bestie DM replies.
 * Distinct from wake-nudge (`!` badge): this is a pulsing light ring for new
 * agent messages while the popover and Bestie DM thread are both closed.
 *
 * Pulse only for real unread outside Bestie surfaces. Stale flags left over
 * from history hydration or system-noise turns must not glow once the user
 * is caught up, and never while a Bestie surface is open.
 */

type Listener = () => void;

const listeners = new Set<Listener>();

let hasUnreadMessage = false;
let popoverOpen = false;
let viewingBestieDm = false;
/** Highest agent message created_at already acknowledged (unix seconds). */
let seenAgentCreatedAt = 0;

function notify() {
  for (const listener of listeners) listener();
}

/**
 * True only when there is real unread AND no Bestie surface is open.
 * Opening the popover / DM must never keep the footer ring glowing.
 */
export function getBestieHasUnreadMessage(): boolean {
  return hasUnreadMessage && !popoverOpen && !viewingBestieDm;
}

export function isBestieSurfaceOpen(): boolean {
  return popoverOpen || viewingBestieDm;
}

export function setBestiePopoverOpen(open: boolean): void {
  if (popoverOpen === open) return;
  popoverOpen = open;
  if (open) {
    clearBestieUnreadMessage();
  }
  notify();
}

export function setBestieViewingDm(viewing: boolean): void {
  if (viewingBestieDm === viewing) return;
  viewingBestieDm = viewing;
  if (viewing) {
    clearBestieUnreadMessage();
  }
  notify();
}

export function clearBestieUnreadMessage(): void {
  if (!hasUnreadMessage) return;
  hasUnreadMessage = false;
  notify();
}

/**
 * Advance the seen watermark without setting (or clearing) unread.
 * Used for hydration and system-noise agent turns that must not pulse.
 */
export function noteBestieAgentMessageCreatedAt(
  createdAtSeconds: number,
): void {
  if (createdAtSeconds > seenAgentCreatedAt) {
    seenAgentCreatedAt = createdAtSeconds;
  }
}

/**
 * Note that the user has caught up through this agent message timestamp.
 * Call when opening Bestie surfaces or when seeding from history.
 */
export function markBestieAgentMessagesSeen(createdAtSeconds: number): void {
  if (createdAtSeconds > seenAgentCreatedAt) {
    seenAgentCreatedAt = createdAtSeconds;
  }
  if (hasUnreadMessage) {
    hasUnreadMessage = false;
    notify();
  }
}

export function getBestieSeenAgentCreatedAt(): number {
  return seenAgentCreatedAt;
}

/**
 * Ingest agent message timestamps. Sets unread when a newer agent message
 * arrives while Bestie surfaces are closed.
 */
export function ingestBestieAgentMessageCreatedAt(
  createdAtSeconds: number,
): void {
  if (createdAtSeconds <= seenAgentCreatedAt) return;
  if (isBestieSurfaceOpen()) {
    seenAgentCreatedAt = createdAtSeconds;
    if (hasUnreadMessage) {
      hasUnreadMessage = false;
      notify();
    }
    return;
  }
  seenAgentCreatedAt = createdAtSeconds;
  if (!hasUnreadMessage) {
    hasUnreadMessage = true;
    notify();
  }
}

export function useBestieHasUnreadMessage(): boolean {
  return React.useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    getBestieHasUnreadMessage,
    () => false,
  );
}

/** Test helper. */
export function __resetBestieAttentionStoreForTests(): void {
  hasUnreadMessage = false;
  popoverOpen = false;
  viewingBestieDm = false;
  seenAgentCreatedAt = 0;
  notify();
}
