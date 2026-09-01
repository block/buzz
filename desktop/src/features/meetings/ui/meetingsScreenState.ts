/**
 * Pure state selection for `MeetingsScreen`. Keeps the branching that decides
 * *what* the screen shows out of the component so it can be unit-tested without
 * a React renderer (Phase 3 test policy: pure helpers only).
 */

import { MeetingError, type MeetingErrorKind } from "@/features/meetings/api";
import type { SubscribeIntent } from "@/features/meetings/api";
import type { MeetingsDeepLinkSearch } from "@/features/meetings/ui/meetingsDeepLink";

/** Same shape the route hands us; see `meetingsDeepLink.ts`. */
export type MeetingsDeepLink = MeetingsDeepLinkSearch;

export type MeetingsView =
  | { kind: "loading" }
  | { kind: "unavailable" }
  | { kind: "call"; room: string }
  | { kind: "list"; prefillRoom?: string; focusStart: boolean };

/**
 * @param hasCapability - relay advertises `buzz-meetings` (capability !== null)
 * @param isCapabilityLoading - capability query still in flight
 */
export function selectMeetingsView(input: {
  hasCapability: boolean;
  isCapabilityLoading: boolean;
  deepLink: MeetingsDeepLink;
}): MeetingsView {
  const { hasCapability, isCapabilityLoading, deepLink } = input;
  if (!hasCapability) {
    return isCapabilityLoading ? { kind: "loading" } : { kind: "unavailable" };
  }
  if (deepLink.action === "join" && deepLink.room) {
    return { kind: "call", room: deepLink.room };
  }
  if (deepLink.action === "start" && deepLink.room) {
    return { kind: "list", prefillRoom: deepLink.room, focusStart: true };
  }
  return { kind: "list", focusStart: false };
}

/** Error kinds that route to the "hosting not enabled" panel (Phase 5 owns the
 * real subscribe / invoice flow). */
const HOSTING_ERROR_KINDS: ReadonlySet<MeetingErrorKind> = new Set([
  "subscription_required",
  "subscription_expired",
  "pending_invoice",
]);

/** True when a failed `registerRoom` should show the hosting panel rather than
 * a transient error + retry. */
export function isHostingSetupError(error: unknown): boolean {
  return error instanceof MeetingError && HOSTING_ERROR_KINDS.has(error.kind);
}

/** The existing invoice carried on a `409 pending_invoice` register/token
 * failure, if the body parsed as one — `SubscribeView` resumes from it. */
export function pendingInvoiceFromError(
  error: unknown,
): SubscribeIntent | undefined {
  return error instanceof MeetingError ? error.pendingInvoice : undefined;
}

/** True when a live badge should render for a room row. */
export function isRoomLive(numParticipants: number | undefined): boolean {
  return (numParticipants ?? 0) > 0;
}
