import type { PresenceStatus } from "@/shared/api/types";

/**
 * Last presence status this client published for itself.
 *
 * `@here` (NIP-CM) only escalates while the reader is actually online, and the
 * predicate that decides this runs deep inside live-event handling where the
 * presence hook's React state is not in scope. A module-level signal keeps the
 * status readable from those pure call sites; `usePresenceSession` is the only
 * writer.
 */
let selfPresenceStatus: PresenceStatus = "offline";

/** Record the current self presence status. Called by `usePresenceSession`. */
export function setSelfPresenceStatus(status: PresenceStatus): void {
  selfPresenceStatus = status;
}

/** Whether this client currently reads as online. */
export function isSelfPresenceOnline(): boolean {
  return selfPresenceStatus === "online";
}

/** Reset to the pre-session default (community switch / sign-out). */
export function resetSelfPresenceStatus(): void {
  selfPresenceStatus = "offline";
}
