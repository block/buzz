import assert from "node:assert/strict";
import test from "node:test";

import { shouldObserveManagedAgents } from "./observerRelayStore.ts";

test("observer ingestion opens for a cold stopped managed agent", () => {
  assert.equal(
    shouldObserveManagedAgents([{ pubkey: "aa", status: "stopped" }]),
    true,
  );
});

test("observer ingestion stays closed when there are no owned agents", () => {
  assert.equal(shouldObserveManagedAgents([]), false);
});
