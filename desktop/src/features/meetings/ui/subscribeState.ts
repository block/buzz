/**
 * Pure state helpers for the Subscribe / payment flow (`SubscribeView`).
 *
 * Keeps the invoice step machine, the BOLT11 → `lightning:` URI builder, the
 * payment-status classification, and the subscription-expiry maths out of the
 * component so they can be unit-tested without a React renderer (Phase 3/4/5
 * test policy: pure helpers only).
 */

import { MeetingError } from "@/features/meetings/api";
import type {
  PaymentStatus,
  SubscribeIntent,
  SubscriptionStatus,
} from "@/features/meetings/api";

/** Where the SubscribeView is in the flow. */
export type SubscribeStep =
  | { kind: "plans" }
  | { kind: "invoice"; intent: SubscribeIntent }
  | { kind: "settled"; subscription?: SubscriptionStatus }
  | { kind: "expired"; plan: string };

/** Terminal state of a payment intent, folded from HiveTalk's raw status text. */
export type PaymentOutcome = "pending" | "settled" | "expired" | "failed";

const SETTLED_STATUSES: ReadonlySet<string> = new Set([
  "settled",
  "paid",
  "complete",
  "completed",
  "succeeded",
  "success",
  "confirmed",
]);

const EXPIRED_STATUSES: ReadonlySet<string> = new Set([
  "expired",
  "timeout",
  "timed_out",
]);

const FAILED_STATUSES: ReadonlySet<string> = new Set([
  "failed",
  "error",
  "cancelled",
  "canceled",
  "void",
]);

/** Fold a raw intent/payment status string onto a terminal-or-pending outcome. */
export function classifyPaymentStatus(
  status: string | undefined,
): PaymentOutcome {
  const normalized = (status ?? "").trim().toLowerCase();
  if (SETTLED_STATUSES.has(normalized)) return "settled";
  if (EXPIRED_STATUSES.has(normalized)) return "expired";
  if (FAILED_STATUSES.has(normalized)) return "failed";
  return "pending";
}

/** True once polling should stop (any outcome that isn't `pending`). */
export function isTerminalPaymentOutcome(outcome: PaymentOutcome): boolean {
  return outcome !== "pending";
}

/**
 * Wallet deep-link for a BOLT11 invoice. Uppercased — the QR alphanumeric mode
 * only covers uppercase, so an uppercased invoice encodes ~45% denser and every
 * spec-compliant wallet lowercases on decode. The copy button always uses the
 * exact string from the relay, never this.
 */
export function buildLightningUri(bolt11: string): string {
  return `lightning:${bolt11.trim().toUpperCase()}`;
}

/** Seconds until `expiresAt` (ISO 8601), clamped at 0. `NaN` timestamp → 0. */
export function secondsUntilExpiry(expiresAt: string, now: number): number {
  const deadline = Date.parse(expiresAt);
  if (Number.isNaN(deadline)) return 0;
  return Math.max(0, Math.floor((deadline - now) / 1000));
}

/** True when the invoice's own `expires_at` is in the past. */
export function isInvoiceExpired(
  intent: SubscribeIntent,
  now: number,
): boolean {
  return secondsUntilExpiry(intent.expires_at, now) <= 0;
}

/** `m:ss` for a countdown; hours fold into minutes (invoices are short-lived). */
export function formatCountdown(totalSeconds: number): string {
  const safe = Math.max(0, Math.floor(totalSeconds));
  const minutes = Math.floor(safe / 60);
  const seconds = safe % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

/**
 * Next step after a `subscribe()` rejection. A `409 pending_invoice` carries the
 * existing intent in `error.pendingInvoice`; surface it instead of erroring.
 * Anything else stays on the plan list (the caller shows the message).
 */
export function stepFromSubscribeError(
  error: unknown,
  plan: string,
): SubscribeStep | null {
  if (
    error instanceof MeetingError &&
    error.kind === "pending_invoice" &&
    error.pendingInvoice
  ) {
    return { kind: "invoice", intent: error.pendingInvoice };
  }
  void plan;
  return null;
}

/** Next step from a fresh payment-status poll while on the invoice step. */
export function stepFromPaymentStatus(
  status: PaymentStatus,
  currentPlan: string,
): SubscribeStep | null {
  switch (classifyPaymentStatus(status.status)) {
    case "settled":
      return { kind: "settled", subscription: status.subscription };
    case "expired":
      return { kind: "expired", plan: status.plan ?? currentPlan };
    case "failed":
      return { kind: "expired", plan: status.plan ?? currentPlan };
    default:
      return null;
  }
}

const NEAR_EXPIRY_DAYS_DEFAULT = 7;
const MS_PER_DAY = 86_400_000;

export type SubscriptionBadgeModel = {
  /** Short status word for the badge pill. */
  label: string;
  /** `entitled` and not close to lapsing. */
  tone: "active" | "warning" | "inactive";
  /** Human expiry line, or null when there's no paid-until date. */
  expiryText: string | null;
  /** Show the "Renew" affordance (expired, in grace, or within the window). */
  showRenew: boolean;
};

/** View model for `SubscriptionStatusBadge`. Pure — formats no dates itself
 * beyond an ISO day slice so it stays locale-stable in tests. */
export function subscriptionBadgeModel(
  subscription: SubscriptionStatus | undefined,
  now: number,
  nearExpiryDays: number = NEAR_EXPIRY_DAYS_DEFAULT,
): SubscriptionBadgeModel | null {
  if (!subscription) return null;

  const paidUntilMs = subscription.paid_until
    ? Date.parse(subscription.paid_until)
    : Number.NaN;
  const hasDate = !Number.isNaN(paidUntilMs);
  const daysLeft = hasDate ? (paidUntilMs - now) / MS_PER_DAY : Number.NaN;
  const expiryText = hasDate
    ? `${daysLeft <= 0 ? "Expired" : "Renews"} ${new Date(paidUntilMs)
        .toISOString()
        .slice(0, 10)}`
    : null;

  if (!subscription.entitled) {
    return {
      label: "No subscription",
      tone: "inactive",
      expiryText,
      showRenew: true,
    };
  }

  const nearExpiry =
    subscription.in_grace || (hasDate && daysLeft <= nearExpiryDays);

  return {
    label: subscription.plan
      ? `${subscription.plan} plan`
      : subscription.in_grace
        ? "In grace period"
        : "Active",
    tone: nearExpiry ? "warning" : "active",
    expiryText,
    showRenew: nearExpiry,
  };
}
