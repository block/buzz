import assert from "node:assert/strict";
import test from "node:test";

import {
  BESTIE_LIST_AGENT_INSTRUCTIONS,
  BESTIE_LIST_TURN_HINT_MARKER,
  stripBestieListTurnHint,
  withBestieListTurnHint,
} from "./bestieListProtocol.ts";

test("agent instructions document the bestie-list fence", () => {
  assert.match(BESTIE_LIST_AGENT_INSTRUCTIONS, /bestie-list/);
  assert.match(BESTIE_LIST_AGENT_INSTRUCTIONS, /"op":"add"/);
  assert.match(BESTIE_LIST_AGENT_INSTRUCTIONS, /dueAt/);
});

test("turn hint attaches and strips cleanly", () => {
  const body = "Remind me to water plants in 10 minutes";
  const withHint = withBestieListTurnHint(body, true);
  assert.match(withHint, new RegExp(BESTIE_LIST_TURN_HINT_MARKER.replace(/[[\]]/g, "\\$&")));
  assert.equal(stripBestieListTurnHint(withHint), body);
  assert.equal(withBestieListTurnHint(body, false), body);
});
