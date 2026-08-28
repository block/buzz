import assert from "node:assert/strict";
import test from "node:test";

import { MeetingError } from "../api.ts";
import {
  buildLightningUri,
  classifyPaymentStatus,
  formatCountdown,
  isInvoiceExpired,
  isTerminalPaymentOutcome,
  secondsUntilExpiry,
  stepFromPaymentStatus,
  stepFromSubscribeError,
  subscriptionBadgeModel,
} from "./subscribeState.ts";

const T0 = Date.parse("2026-08-28T00:00:00.000Z");

function intent(overrides = {}) {
  return {
    intent_id: "int_1",
    plan: "bulk10_1y",
    amount_sats: 21_000,
    bolt11: "lnbc210u1pexample",
    payment_hash: "hash",
    expires_at: new Date(T0 + 10 * 60_000).toISOString(),
    status: "pending",
    ...overrides,
  };
}

test("classifyPaymentStatus folds provider status text", () => {
  assert.equal(classifyPaymentStatus("settled"), "settled");
  assert.equal(classifyPaymentStatus("PAID"), "settled");
  assert.equal(classifyPaymentStatus(" confirmed "), "settled");
  assert.equal(classifyPaymentStatus("expired"), "expired");
  assert.equal(classifyPaymentStatus("failed"), "failed");
  assert.equal(classifyPaymentStatus("canceled"), "failed");
  assert.equal(classifyPaymentStatus("pending"), "pending");
  assert.equal(classifyPaymentStatus(undefined), "pending");
});

test("isTerminalPaymentOutcome: only pending keeps polling", () => {
  assert.equal(isTerminalPaymentOutcome("pending"), false);
  assert.equal(isTerminalPaymentOutcome("settled"), true);
  assert.equal(isTerminalPaymentOutcome("expired"), true);
  assert.equal(isTerminalPaymentOutcome("failed"), true);
});

test("buildLightningUri uppercases the invoice and prefixes the scheme", () => {
  assert.equal(
    buildLightningUri("lnbc210u1pexample"),
    "lightning:LNBC210U1PEXAMPLE",
  );
  assert.equal(buildLightningUri("  lnbc1  "), "lightning:LNBC1");
});

test("secondsUntilExpiry clamps at zero and tolerates bad input", () => {
  assert.equal(secondsUntilExpiry(new Date(T0 + 90_000).toISOString(), T0), 90);
  assert.equal(secondsUntilExpiry(new Date(T0 - 90_000).toISOString(), T0), 0);
  assert.equal(secondsUntilExpiry("not-a-date", T0), 0);
});

test("isInvoiceExpired tracks the intent's own deadline", () => {
  assert.equal(isInvoiceExpired(intent(), T0), false);
  assert.equal(isInvoiceExpired(intent(), T0 + 11 * 60_000), true);
});

test("formatCountdown renders m:ss", () => {
  assert.equal(formatCountdown(0), "0:00");
  assert.equal(formatCountdown(9), "0:09");
  assert.equal(formatCountdown(605), "10:05");
  assert.equal(formatCountdown(-5), "0:00");
});

test("stepFromSubscribeError resumes a pending invoice, else null", () => {
  const err = new MeetingError("pending_invoice", 409, "x");
  err.pendingInvoice = intent();
  assert.deepEqual(stepFromSubscribeError(err, "bulk10_1y"), {
    kind: "invoice",
    intent: intent(),
  });
  assert.equal(
    stepFromSubscribeError(new MeetingError("pending_invoice", 409, "x"), "p"),
    null,
  );
  assert.equal(
    stepFromSubscribeError(
      new MeetingError("provider_unavailable", 503, "x"),
      "p",
    ),
    null,
  );
  assert.equal(stepFromSubscribeError(new Error("nope"), "p"), null);
});

test("stepFromPaymentStatus maps outcomes to the next step", () => {
  assert.deepEqual(
    stepFromPaymentStatus({ intent_id: "i", status: "settled" }, "p"),
    { kind: "settled", subscription: undefined },
  );
  assert.deepEqual(
    stepFromPaymentStatus({ intent_id: "i", status: "expired" }, "fallback"),
    { kind: "expired", plan: "fallback" },
  );
  assert.deepEqual(
    stepFromPaymentStatus(
      { intent_id: "i", status: "failed", plan: "from-status" },
      "fallback",
    ),
    { kind: "expired", plan: "from-status" },
  );
  assert.equal(
    stepFromPaymentStatus({ intent_id: "i", status: "pending" }, "p"),
    null,
  );
});

function subscription(overrides = {}) {
  return {
    status: "active",
    entitled: true,
    room_quota: 10,
    rooms_in_use: 2,
    free_quota: 0,
    grace_days: 7,
    in_grace: false,
    paid_until: "2027-08-21T00:00:00.000Z",
    plan: "bulk10_1y",
    can_record: true,
    pubkey: "abc",
    ...overrides,
  };
}

test("subscriptionBadgeModel: null subscription -> null", () => {
  assert.equal(subscriptionBadgeModel(undefined, T0), null);
});

test("subscriptionBadgeModel: active, far from expiry", () => {
  assert.deepEqual(subscriptionBadgeModel(subscription(), T0), {
    label: "bulk10_1y plan",
    tone: "active",
    expiryText: "Renews 2027-08-21",
    showRenew: false,
  });
});

test("subscriptionBadgeModel: within the near-expiry window -> warning + renew", () => {
  const soon = new Date(T0 + 3 * 86_400_000).toISOString();
  const model = subscriptionBadgeModel(subscription({ paid_until: soon }), T0);
  assert.equal(model.tone, "warning");
  assert.equal(model.showRenew, true);
});

test("subscriptionBadgeModel: not entitled -> inactive + renew", () => {
  const model = subscriptionBadgeModel(
    subscription({ entitled: false, plan: null, paid_until: null }),
    T0,
  );
  assert.deepEqual(model, {
    label: "No subscription",
    tone: "inactive",
    expiryText: null,
    showRenew: true,
  });
});

test("subscriptionBadgeModel: expired paid_until reads 'Expired'", () => {
  const past = new Date(T0 - 86_400_000).toISOString();
  const model = subscriptionBadgeModel(subscription({ paid_until: past }), T0);
  assert.equal(model.expiryText, "Expired 2026-08-27");
  assert.equal(model.tone, "warning");
});
