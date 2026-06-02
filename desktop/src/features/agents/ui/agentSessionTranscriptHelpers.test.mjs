import assert from "node:assert/strict";
import test from "node:test";

import {
  extractPromptText,
  parsePromptText,
} from "./agentSessionTranscriptHelpers.ts";

const HEX = "a".repeat(64);
const HEX_UPPER = "A".repeat(64);

// --- parsePromptText: no section headers ---

test("parsePromptText returns the empty/Prompt fallback for whitespace-only input", () => {
  // The early `sections.length === 0` branch only fires when there are no
  // section bodies at all (e.g. empty/whitespace input).
  const result = parsePromptText("   ");
  assert.deepEqual(result, {
    sections: [],
    userText: "",
    userTitle: "Prompt",
    userPubkey: null,
  });
});

test("parsePromptText wraps header-less free text in a single Prompt section", () => {
  // Free text with no `[header]` becomes one "Prompt" section. Since no
  // section is a "Sprout event", there is no event content to surface, so
  // userText is empty and the title falls through to "Sprout event".
  const result = parsePromptText("just some free text");
  assert.deepEqual(
    result.sections.map((s) => s.title),
    ["Prompt"],
  );
  assert.equal(result.sections[0].body, "just some free text");
  assert.equal(result.userText, "");
  assert.equal(result.userTitle, "Sprout event");
  assert.equal(result.userPubkey, null);
});

// --- parsePromptText: Sprout event section ---

test("parsePromptText extracts content, hex pubkey, and a title-cased kind", () => {
  const text = [
    "[System]",
    "system preamble here",
    "",
    "[Sprout event: @mention]",
    "Channel: demo",
    `From: Wes (hex: ${HEX})`,
    "Content: hello @Brain please look",
  ].join("\n");

  const result = parsePromptText(text);

  assert.equal(result.userText, "hello @Brain please look");
  assert.equal(result.userPubkey, HEX);
  // titleCase capitalizes after word boundaries but leaves the leading "@"
  // (a non-word char) in place: "@mention" -> "@Mention".
  assert.equal(result.userTitle, "@Mention");
  // Both headers become sections.
  assert.deepEqual(
    result.sections.map((s) => s.title),
    ["System", "Sprout event: @mention"],
  );
});

test("parsePromptText lowercases the extracted hex pubkey", () => {
  const text = [
    "[Sprout event: dm]",
    `From: Someone (hex: ${HEX_UPPER})`,
    "Content: hi",
  ].join("\n");

  const result = parsePromptText(text);
  assert.equal(result.userPubkey, HEX);
});

test("parsePromptText yields a null pubkey when From has no hex", () => {
  const text = ["[Sprout event: note]", "From: Someone", "Content: hi"].join(
    "\n",
  );

  const result = parsePromptText(text);
  assert.equal(result.userPubkey, null);
  assert.equal(result.userText, "hi");
  assert.equal(result.userTitle, "Note");
});

test("parsePromptText defaults the title to 'Sprout event' when no kind is present", () => {
  const text = ["[Sprout event]", "Content: x"].join("\n");
  const result = parsePromptText(text);
  assert.equal(result.userTitle, "Sprout event");
});

test("parsePromptText leading text before a header becomes a Prompt section", () => {
  const text = ["preamble line", "[Other]", "body"].join("\n");
  const result = parsePromptText(text);
  assert.deepEqual(
    result.sections.map((s) => s.title),
    ["Prompt", "Other"],
  );
});

// --- extractPromptText ---

test("extractPromptText joins text blocks from params.prompt", () => {
  const payload = {
    params: {
      prompt: [{ text: "line one" }, { text: "line two" }],
    },
  };
  assert.equal(extractPromptText(payload), "line one\nline two");
});

test("extractPromptText handles plain string blocks", () => {
  const payload = { params: { prompt: ["a", "b"] } };
  assert.equal(extractPromptText(payload), "a\nb");
});

test("extractPromptText returns empty string when prompt is missing or not an array", () => {
  assert.equal(extractPromptText({}), "");
  assert.equal(extractPromptText({ params: { prompt: "nope" } }), "");
});
