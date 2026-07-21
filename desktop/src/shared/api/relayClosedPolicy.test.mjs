import assert from "node:assert/strict";
import test from "node:test";

import { classifyRelayClosed } from "./relayClosedPolicy.ts";

// ── classifyRelayClosed ───────────────────────────────────────────────────────

test("classifyRelayClosed: rate-limited messages return rate-limited", () => {
  for (const message of [
    "rate-limited: quota exceeded; retry in 4s",
    "rate-limited: slow down",
    "rate-limited:",
  ]) {
    assert.equal(classifyRelayClosed(message), "rate-limited", message);
  }
});

test("classifyRelayClosed: terminal messages return terminal", () => {
  for (const message of [
    "restricted: not a channel member",
    "restricted: channel access revoked",
    "auth-required: not authenticated",
    "blocked: banned",
    "invalid: malformed filter",
    "pow: difficulty too low",
    "duplicate: subscription exists",
    "unsupported: filter",
    "error: mixed search and non-search filters not supported",
    "error: too many subscriptions",
  ]) {
    assert.equal(classifyRelayClosed(message), "terminal", message);
  }
});

test("classifyRelayClosed: transient errors return retryable", () => {
  for (const message of ["error: database error", "server shutting down", ""]) {
    assert.equal(classifyRelayClosed(message), "retryable", message);
  }
});

// ── Subscription-survival semantics ──────────────────────────────────────────
// These replace the removed isRetryableRelayClosed wrapper tests.
// rate-limited must not delete the subscription; terminal must.

test("classifyRelayClosed: rate-limited class survives (subscription must not be deleted)", () => {
  // Subscription deletion is gated on === "terminal"; rate-limited must survive.
  assert.notEqual(
    classifyRelayClosed("rate-limited: quota exceeded; retry in 4s"),
    "terminal",
  );
});

test("classifyRelayClosed: retryable class survives (subscription must not be deleted)", () => {
  for (const message of ["error: database error", "server shutting down", ""]) {
    assert.notEqual(classifyRelayClosed(message), "terminal", message);
  }
});

test("classifyRelayClosed: terminal class triggers deletion (no retry)", () => {
  for (const message of [
    "restricted: not a channel member",
    "auth-required: not authenticated",
    "blocked: banned",
    "invalid: malformed filter",
    "pow: difficulty too low",
    "duplicate: subscription exists",
    "unsupported: filter",
    "error: mixed search and non-search filters not supported",
    "error: too many subscriptions",
  ]) {
    assert.equal(classifyRelayClosed(message), "terminal", message);
  }
});
