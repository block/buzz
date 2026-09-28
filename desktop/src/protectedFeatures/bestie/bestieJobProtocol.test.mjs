import assert from "node:assert/strict";
import test from "node:test";

import {
  BESTIE_JOB_AGENT_INSTRUCTIONS,
  BESTIE_JOB_TURN_HINT_MARKER,
  stripBestieJobTurnHint,
  withBestieJobTurnHint,
} from "./bestieJobProtocol.ts";

test("agent instructions require confirm-before-create with exact plan", () => {
  assert.match(BESTIE_JOB_AGENT_INSTRUCTIONS, /bestie-job/);
  assert.match(BESTIE_JOB_AGENT_INSTRUCTIONS, /confirmed": true/);
  assert.match(BESTIE_JOB_AGENT_INSTRUCTIONS, /exact approval plan/i);
  assert.match(BESTIE_JOB_AGENT_INSTRUCTIONS, /channel id/i);
  assert.match(BESTIE_JOB_AGENT_INSTRUCTIONS, /do \*\*not\*\* create it immediately/);
});

test("turn hint teaches clarify then approve then confirmed fence", () => {
  const body = "Schedule a job in 5 minutes to summarize my inbox";
  const withHint = withBestieJobTurnHint(body, true);
  assert.match(
    withHint,
    new RegExp(BESTIE_JOB_TURN_HINT_MARKER.replace(/[[\]]/g, "\\$&")),
  );
  assert.match(withHint, /confirmed":true|confirmed:true/);
  assert.match(withHint, /exact plan/i);
  assert.equal(stripBestieJobTurnHint(withHint), body);
  assert.equal(withBestieJobTurnHint(body, false), body);
});
