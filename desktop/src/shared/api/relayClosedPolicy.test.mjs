import assert from "node:assert/strict";
import test from "node:test";

import {
  classifyRelayClosed,
  isRetryableRelayClosed,
} from "./relayClosedPolicy.ts";

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

// ── isRetryableRelayClosed (legacy wrapper) ───────────────────────────────────

test("isRetryableRelayClosed: rate-limited is retryable (subscription must survive)", () => {
  assert.equal(
    isRetryableRelayClosed("rate-limited: quota exceeded; retry in 4s"),
    true,
  );
});

test("isRetryableRelayClosed: transient CLOSED responses are retryable", () => {
  for (const message of ["error: database error", "server shutting down", ""]) {
    assert.equal(isRetryableRelayClosed(message), true, message);
  }
});

test("isRetryableRelayClosed: permanent CLOSED responses are not retryable", () => {
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
    assert.equal(isRetryableRelayClosed(message), false, message);
  }
});
