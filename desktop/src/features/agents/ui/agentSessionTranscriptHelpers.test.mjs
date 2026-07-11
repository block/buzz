import assert from "node:assert/strict";
import test from "node:test";

import {
  extractPromptText,
  extractToolIdentity,
  parsePromptText,
  parseSystemPromptSections,
} from "./agentSessionTranscriptHelpers.ts";

const HEX = "a".repeat(64);
const HEX_UPPER = "A".repeat(64);

test("parsePromptText returns the empty/Prompt fallback for whitespace-only input", () => {
  // The early `sections.length === 0` branch only fires when there are no
  // section bodies at all (e.g. empty/whitespace input).
  const result = parsePromptText("   ");
  assert.deepEqual(result, {
    sections: [],
    userText: "",
    userTitle: "Prompt",
    userPubkey: null,
    userEventId: null,
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
  assert.equal(result.userEventId, null);
});

test("parsePromptText extracts event id, content, hex pubkey, and a title-cased kind", () => {
  const text = [
    "[System]",
    "system preamble here",
    "",
    "[Buzz event: @mention]",
    `Event ID: ${HEX_UPPER}`,
    "Channel: demo",
    `From: Wes (hex: ${HEX})`,
    "Content: hello @Brain please look",
  ].join("\n");

  const result = parsePromptText(text);

  assert.equal(result.userText, "hello @Brain please look");
  assert.equal(result.userPubkey, HEX);
  assert.equal(result.userEventId, HEX);
  // titleCase capitalizes after word boundaries but leaves the leading "@"
  // (a non-word char) in place: "@mention" -> "@Mention".
  assert.equal(result.userTitle, "@Mention");
  // Both headers become sections.
  assert.deepEqual(
    result.sections.map((s) => s.title),
    ["System", "Buzz event: @mention"],
  );
});

test("parsePromptText preserves multiline event content in the user bubble text", () => {
  const text = [
    "[Buzz event: @mention]",
    "Event ID: event-1",
    "Channel: agents",
    `From: tho (hex: ${HEX})`,
    "Time: 2026-06-15T17:15:00Z",
    "Content: @Ned",
    "",
    "- remove that stray cherry pick if it's not adding value here",
    "- help me understand what that e2eBridge change does",
    "- we'd want the e2e seed path as a separate pull request",
    'Tags: [["h","agents"]]',
    "Parsed: mentions=[Ned]",
  ].join("\n");

  const result = parsePromptText(text);

  assert.equal(
    result.userText,
    [
      "@Ned",
      "",
      "- remove that stray cherry pick if it's not adding value here",
      "- help me understand what that e2eBridge change does",
      "- we'd want the e2e seed path as a separate pull request",
    ].join("\n"),
  );
  assert.equal(result.userPubkey, HEX);
  assert.equal(result.userEventId, null);
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

test("extractToolIdentity ignores Buzz tool names that only appear in file contents", () => {
  const identity = extractToolIdentity({
    sessionUpdate: "tool_call_update",
    toolCallId: "read-file-1",
    status: "completed",
    title: "read_file",
    kind: "read_file",
    rawInput: {
      path: "desktop/src/features/agents/ui/agentSessionToolCatalog.ts",
    },
    content: {
      text: 'const BUZZ_READ_TOOLS = new Set(["get_feed", "get_event"]);',
    },
  });

  assert.deepEqual(identity, {
    title: "read_file",
    toolName: "read_file",
    buzzToolName: null,
  });
});

test("extractToolIdentity still recognizes explicit Buzz tool fields", () => {
  const identity = extractToolIdentity({
    sessionUpdate: "tool_call",
    title: "Tool call",
    toolName: "get_feed",
    rawInput: { limit: 50 },
  });

  assert.deepEqual(identity, {
    title: "Tool call",
    toolName: "get_feed",
    buzzToolName: "get_feed",
  });
});

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

// ── [Agent Memory — core] section tests ──────────────────────────────────────

test("parseSystemPromptSections extracts core as its own section after Base+System", () => {
  const framed =
    "[Base]\nbase text\n\n[System]\npersona text\n\n[Agent Memory — core]\nmy memories";
  const sections = parseSystemPromptSections(framed);
  assert.deepEqual(sections, [
    { title: "Base", body: "base text" },
    { title: "System", body: "persona text" },
    { title: "Core Memory", body: "my memories" },
  ]);
});

test("parseSystemPromptSections extracts core as its own section after Base only", () => {
  const framed = "[Base]\nbase text\n\n[Agent Memory — core]\nmy memories";
  const sections = parseSystemPromptSections(framed);
  assert.deepEqual(sections, [
    { title: "Base", body: "base text" },
    { title: "Core Memory", body: "my memories" },
  ]);
});

test("parseSystemPromptSections extracts core as its own section after System only", () => {
  const framed = "[System]\npersona text\n\n[Agent Memory — core]\nmy memories";
  const sections = parseSystemPromptSections(framed);
  assert.deepEqual(sections, [
    { title: "System", body: "persona text" },
    { title: "Core Memory", body: "my memories" },
  ]);
});

test("parseSystemPromptSections returns only a core section when no Base/System present", () => {
  const framed = "[Agent Memory — core]\nmy memories";
  const sections = parseSystemPromptSections(framed);
  assert.deepEqual(sections, [{ title: "Core Memory", body: "my memories" }]);
});

test("parseSystemPromptSections keeps an embedded core-like line literal when a real appended core follows", () => {
  // The persona body contains a line that looks like the header. The actual
  // appended core block comes last — only the LAST boundary should split.
  const framed = [
    "[Base]",
    "base text",
    "",
    "[System]",
    "persona preamble",
    "[Agent Memory — core]",
    "this is NOT the core section — it is inside the persona body",
    "",
    "[Agent Memory — core]",
    "this IS the appended core",
  ].join("\n");
  const sections = parseSystemPromptSections(framed);
  assert.deepEqual(sections, [
    { title: "Base", body: "base text" },
    {
      title: "System",
      body: "persona preamble\n[Agent Memory — core]\nthis is NOT the core section — it is inside the persona body",
    },
    {
      title: "Core Memory",
      body: "this IS the appended core",
    },
  ]);
});

test("parseSystemPromptSections keeps exact core header literal when only a single newline precedes it (no-core persona)", () => {
  // A no-core [System] persona that contains the exact header text on its own
  // line (preceded by only a single \n, not the double-newline appended
  // separator) must NOT be extracted as a Core Memory section.
  const framed = [
    "[System]",
    "persona preamble",
    "[Agent Memory — core]",
    "this is persona text, not a real core block",
  ].join("\n");
  const sections = parseSystemPromptSections(framed);
  assert.deepEqual(sections, [
    {
      title: "System",
      body: "persona preamble\n[Agent Memory — core]\nthis is persona text, not a real core block",
    },
  ]);
});

test("parseSystemPromptSections pins the realistic Workspace+Base+System+Core harness shape", () => {
  // The real Buzz harness emits [Workspace] content before [Base]. The parser
  // folds [Workspace] into the Base section (existing unchanged behavior);
  // core is extracted as a distinct "Core Memory" section last.
  const framed = [
    "[Workspace]",
    "You are operating inside the Buzz platform.",
    "",
    "[Base]",
    "You are an assistant.",
    "",
    "[System]",
    "Custom persona instructions.",
    "",
    "[Agent Memory — core]",
    "I am Duncan.",
    "## Lessons Learned",
    "Always tag on handoff.",
  ].join("\n");
  const sections = parseSystemPromptSections(framed);
  assert.deepEqual(sections, [
    {
      title: "Base",
      body: "[Workspace]\nYou are operating inside the Buzz platform.\n\n[Base]\nYou are an assistant.",
    },
    { title: "System", body: "Custom persona instructions." },
    {
      title: "Core Memory",
      body: "I am Duncan.\n## Lessons Learned\nAlways tag on handoff.",
    },
  ]);
});
