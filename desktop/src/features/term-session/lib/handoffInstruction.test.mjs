import assert from "node:assert/strict";
import test from "node:test";

import { buildTermSessionHandoffInstruction } from "./handoffInstruction.ts";

const CHANNEL = "f570339f-8f8a-4e08-a779-8d954aa44109";
const MESSAGE =
  "b04819ffc1f7c8ffb49c6d30b5899f470198264680d02e78894a658e30a9059f";
const THREAD =
  "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";

test("slim handoff points at term_session_card and keeps context", () => {
  const text = buildTermSessionHandoffInstruction({
    agentDisplayName: "Go",
    channelId: "ch-1",
    threadId: "th-2",
    harness: "claude",
  });
  assert.match(text, /^@Go /);
  assert.match(text, /buzz-dev-mcp tool `term_session_card`/);
  assert.match(text, /channelId: ch-1/);
  assert.match(text, /threadId: th-2/);
  assert.match(text, /harness: claude/);
  assert.match(text, /Omit openclawWorkspace/);
  assert.doesNotMatch(text, /"hula": "term-session"/);
  assert.doesNotMatch(text, /"v": 1/);
  assert.match(text, /paste the tool output exactly/);
  assert.match(text, /progress stays in the TUI/);
  assert.match(text, /buzz_draft_message/);
});

test("includes filled Return path with origin and selected-agent mention", () => {
  const text = buildTermSessionHandoffInstruction({
    agentDisplayName: "Fable",
    channelId: "ch-origin",
    threadId: "th-origin",
    harness: "claude",
    mentionPubkey: "pk-fable",
  });
  assert.match(text, /## Return path \(Buzz\)/);
  assert.match(text, /origin channelId: ch-origin/);
  assert.match(text, /origin threadId:  th-origin/);
  assert.match(text, /mention to use:   @Fable/);
  assert.match(text, /mention pubkey:   pk-fable/);
  assert.match(text, /originChannelId: ch-origin/);
  assert.match(text, /mentionToUse: @Fable/);
  assert.match(text, /mentionPubkey: pk-fable/);
  assert.match(text, /report back/);
  assert.match(text, /hand back/);
  assert.match(text, /I'm done/);
  assert.match(text, /mentions:/);
  assert.match(text, /Plain @Name alone is NOT enough/);
  assert.match(text, /Append this exact Return path block/);
});

test("origin deep link overrides channelId/threadId for Return path", () => {
  const text = buildTermSessionHandoffInstruction({
    agentDisplayName: "ClaimMiner",
    channelId: "wrong-ch",
    threadId: "wrong-th",
    harness: "codex",
    originDeepLink: `hulabuzz://message?channel=${CHANNEL}&id=${MESSAGE}&thread=${THREAD}`,
  });
  assert.match(text, new RegExp(`origin channelId: ${CHANNEL}`));
  assert.match(text, new RegExp(`origin threadId:  ${THREAD}`));
  assert.match(text, /mention to use:   @ClaimMiner/);
  // Summary context still shows the caller's channel/thread props.
  assert.match(text, /channelId: wrong-ch/);
  assert.match(text, /threadId: wrong-th/);
});

test("labels summarized/source when distinct from origin", () => {
  const text = buildTermSessionHandoffInstruction({
    agentDisplayName: "Fable",
    channelId: "ch-origin",
    threadId: "th-origin",
    harness: "claude",
    summarizedChannelId: "ch-origin",
    summarizedThreadId: "th-summarized",
  });
  assert.match(text, /summarized\/source threadId:  th-summarized/);
  assert.match(text, /origin threadId:  th-origin/);
});

test("mentionToUse override wins over agent display name", () => {
  const text = buildTermSessionHandoffInstruction({
    agentDisplayName: "Fable",
    channelId: "c",
    threadId: "t",
    harness: "claude",
    mentionToUse: "@Ship",
  });
  assert.match(text, /mention to use:   @Ship/);
  assert.match(text, /^@Fable /);
});

test("openclaw flag asks for boolean true only", () => {
  const text = buildTermSessionHandoffInstruction({
    agentDisplayName: "Bot",
    channelId: "c",
    threadId: "t",
    harness: "codex",
    openclawWorkspace: true,
  });
  assert.match(text, /openclawWorkspace: true/);
  assert.match(text, /boolean only/);
  assert.match(text, /never put tokens\/JWTs/);
  assert.match(text, /tool \/ harness: codex/);
});

test("falls back to @agent when display name blank", () => {
  const text = buildTermSessionHandoffInstruction({
    agentDisplayName: "  ",
    channelId: "c",
    threadId: "t",
    harness: "claude",
  });
  assert.match(text, /^@agent /);
  assert.match(text, /mention to use:   @agent/);
});
