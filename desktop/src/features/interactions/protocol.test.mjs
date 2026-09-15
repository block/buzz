import assert from "node:assert/strict";
import test from "node:test";
import { parseInteractionPrompt, parseInteractionState } from "./protocol.ts";
const relay = "a".repeat(64);
const prompt = "b".repeat(64);
const channel = "00000000-0000-0000-0000-000000000001";
const state = {
  id: "c".repeat(64),
  pubkey: relay,
  kind: 39010,
  created_at: 1,
  tags: [
    ["d", prompt],
    ["h", channel],
  ],
  content: JSON.stringify({
    version: 1,
    revision: 2,
    status: "closed",
    close_reason: "first",
    winner: "yes",
    tally: { yes: 1 },
    responders: [
      {
        pubkey: "d".repeat(64),
        event_id: "e".repeat(64),
        created_at: 1,
        choices: ["yes"],
      },
    ],
  }),
  sig: "",
};
test("only matching relay state can authorize a decision card", () => {
  assert.equal(
    parseInteractionState(state, prompt, channel, relay)?.winner,
    "yes",
  );
  for (const patch of [
    { pubkey: "f".repeat(64) },
    { kind: 40011 },
    {
      tags: [
        ["d", "f".repeat(64)],
        ["h", channel],
      ],
    },
    {
      tags: [
        ["d", prompt],
        ["h", "other"],
      ],
    },
    { content: "{" },
    {
      content: JSON.stringify({
        version: 1,
        revision: -1,
        status: "open",
        responders: [],
        tally: {},
      }),
    },
  ])
    assert.equal(
      parseInteractionState({ ...state, ...patch }, prompt, channel, relay),
      null,
    );
});
test("unsupported form privacy and fields fail closed", () => {
  const event = {
    ...state,
    id: prompt,
    kind: 40010,
    tags: [
      ["h", channel],
      ["itype", "form"],
      ["field", "note", "Note", "text", "required"],
      ["deadline", "2000"],
    ],
    content: "Details?",
  };
  assert.equal(
    parseInteractionPrompt(event, prompt, channel).fields[0].type,
    "text",
  );
  assert.throws(() =>
    parseInteractionPrompt(
      { ...event, tags: [...event.tags, ["visibility", "asker-only"]] },
      prompt,
      channel,
    ),
  );
  assert.throws(() =>
    parseInteractionPrompt(
      {
        ...event,
        tags: [...event.tags, ["field", "key", "Key", "secret", "required"]],
      },
      prompt,
      channel,
    ),
  );
});
