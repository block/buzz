import assert from "node:assert/strict";
import test from "node:test";

import { resolveCreateIntent } from "./agentCreateIntent.ts";

test("resolveCreateIntent defaults to quick-start for un-migrated callers", () => {
  // PersonaDialog's duplicate path calls handleSubmit without an intent until
  // B3 migrates it; the default must preserve today's create-then-start
  // behavior or duplicate silently becomes definition-only.
  assert.equal(resolveCreateIntent(undefined), "definition_start");
});

test("resolveCreateIntent passes explicit intents through", () => {
  assert.equal(resolveCreateIntent("definition"), "definition");
  assert.equal(resolveCreateIntent("definition_start"), "definition_start");
});
