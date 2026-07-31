import assert from "node:assert/strict";
import test from "node:test";

import { selectLatestActivityHeadline } from "./agentActivityStatus.ts";

const CHANNEL = "channel-1";
const OTHER_CHANNEL = "channel-2";

let nextId = 0;

function thought(title, overrides = {}) {
  nextId += 1;
  return {
    id: `item-${nextId}`,
    type: "thought",
    renderClass: "thought",
    title,
    text: title,
    timestamp: "2026-07-31T12:00:00Z",
    channelId: CHANNEL,
    ...overrides,
  };
}

function assistantMessage(text, overrides = {}) {
  nextId += 1;
  return {
    id: `item-${nextId}`,
    type: "message",
    renderClass: "message",
    role: "assistant",
    title: "Assistant",
    text,
    timestamp: "2026-07-31T12:00:00Z",
    channelId: CHANNEL,
    ...overrides,
  };
}

function userPrompt(overrides = {}) {
  nextId += 1;
  return {
    id: `item-${nextId}`,
    type: "message",
    renderClass: "message",
    role: "user",
    title: "User prompt",
    text: "please do the thing",
    timestamp: "2026-07-31T12:00:00Z",
    channelId: CHANNEL,
    ...overrides,
  };
}

function metadata(title, overrides = {}) {
  nextId += 1;
  return {
    id: `item-${nextId}`,
    type: "metadata",
    renderClass: "raw-rail",
    title,
    sections: [],
    timestamp: "2026-07-31T12:00:00Z",
    channelId: CHANNEL,
    ...overrides,
  };
}

test("selectLatestActivityHeadline_newestSpineItemWins", () => {
  const headline = selectLatestActivityHeadline(
    [thought("Reading the config"), thought("Editing the parser")],
    CHANNEL,
  );
  assert.equal(headline, "Editing the parser");
});

test("selectLatestActivityHeadline_scopedToChannel", () => {
  const headline = selectLatestActivityHeadline(
    [
      thought("Working here"),
      thought("Working elsewhere", { channelId: OTHER_CHANNEL }),
    ],
    CHANNEL,
  );
  assert.equal(headline, "Working here");
});

test("selectLatestActivityHeadline_ignoresUserPromptEcho", () => {
  const headline = selectLatestActivityHeadline(
    [thought("Running tests"), userPrompt()],
    CHANNEL,
  );
  assert.equal(headline, "Running tests");
});

test("selectLatestActivityHeadline_metadataOnlyFallsBack", () => {
  // No spine work yet (turn start) — metadata reads may headline.
  const headline = selectLatestActivityHeadline(
    [metadata("Prompt context")],
    CHANNEL,
  );
  assert.equal(headline, "Prompt context");
});

test("selectLatestActivityHeadline_spinePresenceHidesMetadata", () => {
  const headline = selectLatestActivityHeadline(
    [thought("Planning"), metadata("Prompt context")],
    CHANNEL,
  );
  assert.equal(headline, "Planning");
});

test("selectLatestActivityHeadline_assistantMessageHeadlinesFirstLine", () => {
  const headline = selectLatestActivityHeadline(
    [assistantMessage("Done — pushed the fix.\nDetails below.")],
    CHANNEL,
  );
  assert.equal(headline, "Done — pushed the fix.");
});

test("selectLatestActivityHeadline_emptyTranscriptIsNull", () => {
  assert.equal(selectLatestActivityHeadline([], CHANNEL), null);
  assert.equal(selectLatestActivityHeadline([userPrompt()], CHANNEL), null);
});
