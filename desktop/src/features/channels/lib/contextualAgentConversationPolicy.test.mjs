/**
 * Red: shared contextual-agent fixture must match Desktop policy decisions.
 * Expected: every case fails until resolveContextualAgentConversation is
 * implemented (Green leaf).
 */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import path from "node:path";

import { resolveContextualAgentConversation } from "./contextualAgentConversationPolicy.ts";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const fixturePath = path.resolve(
  __dirname,
  "../../../../../tests/fixtures/contextual-agent-conversation-cases.json",
);
const fixture = JSON.parse(readFileSync(fixturePath, "utf8"));

assert.equal(fixture.version, 1);
assert.ok(Array.isArray(fixture.cases));
assert.ok(fixture.cases.length >= 12);

function sorted(list) {
  return [...list].sort();
}

function assertDecision(actual, expected, caseId) {
  assert.deepEqual(
    sorted(actual.audiencePubkeys),
    sorted(expected.audiencePubkeys),
    `${caseId}: audiencePubkeys`,
  );
  assert.deepEqual(
    actual.replyPlacement,
    expected.replyPlacement,
    `${caseId}: replyPlacement`,
  );
  assert.equal(
    actual.sharedThread,
    expected.sharedThread,
    `${caseId}: sharedThread`,
  );
  assert.equal(
    actual.retainDraft,
    expected.retainDraft,
    `${caseId}: retainDraft`,
  );
  if (expected.nestUnderAgentReply !== undefined) {
    assert.equal(
      actual.nestUnderAgentReply ?? false,
      expected.nestUnderAgentReply,
      `${caseId}: nestUnderAgentReply`,
    );
  }
}

for (const c of fixture.cases) {
  test(`contextual fixture (desktop): ${c.id}`, () => {
    const input = {
      ...c.input,
      humanMessageEventId:
        c.expected.replyPlacement?.kind === "thread-root" &&
        c.expected.replyPlacement.eventId === "human-message-id"
          ? "human-message-id"
          : (c.input.humanMessageEventId ?? null),
    };
    const decision = resolveContextualAgentConversation(input);
    assertDecision(decision, c.expected, c.id);
  });
}
