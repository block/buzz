import assert from "node:assert/strict";
import test from "node:test";

import { isRetryableRelayClosed } from "./relayClosedPolicy.ts";

test("retries transient CLOSED responses", () => {
  for (const message of [
    "rate-limited: slow down",
    "error: database error",
    "server shutting down",
    "",
  ]) {
    assert.equal(isRetryableRelayClosed(message), true, message);
  }
});

test("does not retry permanent CLOSED responses", () => {
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
    assert.equal(isRetryableRelayClosed(message), false, message);
  }
});
