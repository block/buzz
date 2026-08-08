import assert from "node:assert/strict";
import test from "node:test";

import {
  describeAgentReadinessFailures,
  mergeMentionRecipients,
} from "./useMentionSendFlow.helpers.ts";

test("describeAgentReadinessFailures: no failures blocks nothing and warns about nothing", () => {
  assert.deepEqual(describeAgentReadinessFailures([]), {
    blocking: null,
    warning: null,
  });
});

test("describeAgentReadinessFailures: a launch failure for a channel member only warns", () => {
  // #5099: the agent runs on the user's own server, so this desktop holds no
  // key for it and the launch can never succeed. It is in the channel and
  // answers there, so the mention must still go out.
  const result = describeAgentReadinessFailures([
    { blocking: false, message: "Backend · Claude: agent has no private key" },
  ]);

  assert.equal(result.blocking, null);
  assert.equal(
    result.warning,
    "Could not start the mentioned agent; sending the mention anyway: Backend · Claude: agent has no private key",
  );
});

test("describeAgentReadinessFailures: the warning never claims the message was sent", () => {
  // The notice is emitted before Huddle sync, media upload and the send, each
  // of which can still abort. Past-tense copy here would be a false delivery
  // confirmation on an error path.
  for (const count of [1, 2]) {
    const { warning } = describeAgentReadinessFailures(
      Array.from({ length: count }, (_, index) => ({
        blocking: false,
        message: `Agent ${index}: no private key`,
      })),
    );

    assert.match(warning, /sending the mention anyway/);
    assert.doesNotMatch(warning, /\bsent\b/i);
    assert.doesNotMatch(warning, /\bdelivered\b/i);
  }
});

test("describeAgentReadinessFailures: a blocking failure keeps stopping the send", () => {
  const result = describeAgentReadinessFailures([
    { blocking: true, message: "Fizz: Mock agent startup failed." },
  ]);

  assert.equal(
    result.blocking,
    "Could not start agent mention: Fizz: Mock agent startup failed.",
  );
  assert.equal(result.warning, null);
});

test("describeAgentReadinessFailures: a blocking failure wins over a warning", () => {
  const result = describeAgentReadinessFailures([
    { blocking: false, message: "Remote: no private key" },
    { blocking: true, message: "Fizz: startup failed" },
  ]);

  assert.equal(
    result.blocking,
    "Could not start agent mention: Fizz: startup failed",
  );
  assert.equal(
    result.warning,
    "Could not start the mentioned agent; sending the mention anyway: Remote: no private key",
  );
});

test("describeAgentReadinessFailures: several failures of one kind are joined and pluralised", () => {
  const result = describeAgentReadinessFailures([
    { blocking: true, message: "A: one" },
    { blocking: true, message: "B: two" },
    { blocking: false, message: "C: three" },
    { blocking: false, message: "D: four" },
  ]);

  assert.equal(
    result.blocking,
    "Could not start agent mentions: A: one; B: two",
  );
  assert.equal(
    result.warning,
    "Could not start the mentioned agents; sending the mention anyway: C: three; D: four",
  );
});

test("address-locked agents join explicit mentions without duplicating recipients", () => {
  const explicit = ["A".repeat(64), "b".repeat(64)];
  const locked = ["a".repeat(64), "C".repeat(64)];

  assert.deepEqual(mergeMentionRecipients(explicit, locked), [
    "a".repeat(64),
    "b".repeat(64),
    "c".repeat(64),
  ]);
});
