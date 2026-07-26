import assert from "node:assert/strict";
import test from "node:test";

import {
  promptUpdateFrame,
  shouldPushAfterSubmit,
} from "./serverPromptUpdatePush.ts";

const context = {
  spawnerPubkey: "sp",
  specSlug: "fizz",
  agentPubkey: "ag",
  spawnerName: "host",
};

test("a failed save is not pushed", () => {
  assert.equal(shouldPushAfterSubmit(false), false);
  // The dialogs' onSubmit is typed `unknown`; anything but an explicit false
  // is treated as a successful save.
  assert.equal(shouldPushAfterSubmit(true), true);
  assert.equal(shouldPushAfterSubmit(undefined), true);
});

test("a successful save enqueues trimmed material with blanks dropped", () => {
  assert.deepEqual(
    promptUpdateFrame(context, {
      systemPrompt: "  be Fizz  ",
      model: "   ",
      provider: null,
    }),
    {
      spawnerPubkey: "sp",
      specSlug: "fizz",
      agentPubkey: "ag",
      prompt: {
        system_prompt: "be Fizz",
        model: undefined,
        provider: undefined,
      },
    },
  );
});

test("an all-empty edit is not enqueued", () => {
  // The spawner drops an empty update without acking it, so queueing one would
  // livelock: pending forever, resent (and restarting the container) forever.
  assert.equal(
    promptUpdateFrame(context, {
      systemPrompt: "   ",
      model: "",
      provider: null,
    }),
    null,
  );
});

test("a non-server agent is not enqueued", () => {
  assert.equal(
    promptUpdateFrame(null, {
      systemPrompt: "be Fizz",
      model: "m",
      provider: "p",
    }),
    null,
  );
});
