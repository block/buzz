import assert from "node:assert/strict";
import test from "node:test";

import { buildTermSessionHandoffInstruction } from "./handoffInstruction.ts";

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
});
