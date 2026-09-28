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
  assert.match(BESTIE_LIST_AGENT_INSTRUCTIONS, /milliseconds/);
});

test("agent instructions teach crystallize + bare clock confirm", () => {
  assert.match(BESTIE_LIST_AGENT_INSTRUCTIONS, /crystallized/i);
  assert.match(BESTIE_LIST_AGENT_INSTRUCTIONS, /Run the nightly report/);
  assert.match(BESTIE_LIST_AGENT_INSTRUCTIONS, /Bare clock without AM\/PM/);
  assert.match(BESTIE_LIST_AGENT_INSTRUCTIONS, /same thread/);
  assert.match(BESTIE_LIST_AGENT_INSTRUCTIONS, /top-level Assistant DM/);
  assert.match(BESTIE_LIST_AGENT_INSTRUCTIONS, /Ask AM vs PM/);
  assert.match(BESTIE_LIST_AGENT_INSTRUCTIONS, /Bestie live lists/);
  assert.match(BESTIE_LIST_AGENT_INSTRUCTIONS, /cannot query live state/);
});

test("turn hint attaches and strips cleanly", () => {
  const body = "Remind me to water plants in 10 minutes";
  const withHint = withBestieListTurnHint(body, true);
  assert.match(
    withHint,
    new RegExp(BESTIE_LIST_TURN_HINT_MARKER.replace(/[[\]]/g, "\\$&")),
  );
  assert.match(withHint, /crystallize/i);
  assert.equal(stripBestieListTurnHint(withHint), body);
  assert.equal(withBestieListTurnHint(body, false), body);
});

test("bare-clock turn hint strengthens confirm-before-create", () => {
  const body = "Remind me to run the nightly report at 8:36";
  const withHint = withBestieListTurnHint(body, true, {
    bareClockConfirm: true,
  });
  assert.match(withHint, /do NOT create a reminder yet/i);
  assert.match(withHint, /Ask AM vs PM/);
  assert.match(withHint, /same thread/);
  assert.match(withHint, /top-level Assistant DM/);
  assert.match(withHint, /Run the nightly report/);
  assert.equal(stripBestieListTurnHint(withHint), body);
});
