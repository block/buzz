import assert from "node:assert/strict";
import test from "node:test";

import {
  extractPromptText,
  parsePromptText,
  parseSystemPromptSections,
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
  // section is a "Buzz event", there is no event content to surface, so
  // userText is empty and the title falls through to "Buzz event".
  const result = parsePromptText("just some free text");
  assert.deepEqual(
    result.sections.map((s) => s.title),
    ["Prompt"],
  );
  assert.equal(result.sections[0].body, "just some free text");
  assert.equal(result.userText, "");
  assert.equal(result.userTitle, "Buzz event");
  assert.equal(result.userPubkey, null);
});

// --- parsePromptText: Buzz event section ---

test("parsePromptText extracts content, hex pubkey, and a title-cased kind", () => {
  const text = [
    "[System]",
    "system preamble here",
    "",
    "[Buzz event: @mention]",
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
    ["System", "Buzz event: @mention"],
  );
});

test("parsePromptText lowercases the extracted hex pubkey", () => {
  const text = [
    "[Buzz event: dm]",
    `From: Someone (hex: ${HEX_UPPER})`,
    "Content: hi",
  ].join("\n");

  const result = parsePromptText(text);
  assert.equal(result.userPubkey, HEX);
});

test("parsePromptText yields a null pubkey when From has no hex", () => {
  const text = ["[Buzz event: note]", "From: Someone", "Content: hi"].join(
    "\n",
  );

  const result = parsePromptText(text);
  assert.equal(result.userPubkey, null);
  assert.equal(result.userText, "hi");
  assert.equal(result.userTitle, "Note");
});

test("parsePromptText defaults the title to 'Buzz event' when no kind is present", () => {
  const text = ["[Buzz event]", "Content: x"].join("\n");
  const result = parsePromptText(text);
  assert.equal(result.userTitle, "Buzz event");
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

// --- parseSystemPromptSections: deterministic Base/System split ---

test("parseSystemPromptSections splits both prompts into Base and System", () => {
  const framed = "[Base]\nbase text\n\n[System]\npersona text";
  const sections = parseSystemPromptSections(framed);
  assert.deepEqual(sections, [
    { title: "Base", body: "base text" },
    { title: "System", body: "persona text" },
  ]);
});

test("parseSystemPromptSections yields one Base section for a base-only frame", () => {
  const sections = parseSystemPromptSections("[Base]\nbase text");
  assert.deepEqual(sections, [{ title: "Base", body: "base text" }]);
});

test("parseSystemPromptSections yields one System section for a persona-only frame", () => {
  const sections = parseSystemPromptSections("[System]\npersona text");
  assert.deepEqual(sections, [{ title: "System", body: "persona text" }]);
});

test("parseSystemPromptSections keeps embedded bracket lines literal in bodies", () => {
  // A persona that itself contains a [Context]-like line must NOT split into a
  // spurious sub-section — the body is read literally after the first boundary.
  const framed = "[Base]\nbase\n\n[System]\nrule one\n[Context]\nrule two";
  const sections = parseSystemPromptSections(framed);
  assert.deepEqual(sections, [
    { title: "Base", body: "base" },
    { title: "System", body: "rule one\n[Context]\nrule two" },
  ]);
});

test("parseSystemPromptSections degrades to a labeled Base when [System] header is elided", () => {
  // Oversize trim can drop the [System] header mid-string. Without a boundary
  // the whole value stays under a correctly-labeled Base — no missing label,
  // no inflated count, just a truncated body.
  const elided = "[Base]\nbase text …[elided 900000 bytes]… persona tail";
  const sections = parseSystemPromptSections(elided);
  assert.deepEqual(sections, [
    { title: "Base", body: "base text …[elided 900000 bytes]… persona tail" },
  ]);
});

test("parseSystemPromptSections returns no sections for empty input", () => {
  assert.deepEqual(parseSystemPromptSections(""), []);
  assert.deepEqual(parseSystemPromptSections("   "), []);
});
