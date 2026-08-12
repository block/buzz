import assert from "node:assert/strict";
import test from "node:test";

import { describeLogFile } from "./agentUi.ts";

test("log display uses the canonical npub DTO label instead of the storage filename", () => {
  const hex = "f".repeat(64);
  const npubLabel = "npub1canonicalidentity.log";

  assert.equal(describeLogFile(`/tmp/${hex}.log`, npubLabel), npubLabel);
});

test("log display retains the legacy fallback when no display label is available", () => {
  assert.equal(describeLogFile("/tmp/harness.log"), "harness.log");
});
