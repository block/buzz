import assert from "node:assert/strict";
import test from "node:test";

import {
  agentProgressIsActive,
  agentProgressLatestLabel,
  isAgentProgressBody,
} from "./agentProgressMessages.ts";

test("detects_cursor_bridge_progress_lines", () => {
  assert.equal(isAgentProgressBody("▸ Working · grok"), true);
  assert.equal(isAgentProgressBody("⏳ git status"), true);
  assert.equal(isAgentProgressBody("⚙ shell — ls"), true);
  assert.equal(isAgentProgressBody("✓ shell"), true);
  assert.equal(isAgentProgressBody("✗ grep"), true);
  assert.equal(isAgentProgressBody("✅ Done"), true);
  assert.equal(isAgentProgressBody("Here is the real answer."), false);
  assert.equal(isAgentProgressBody(""), false);
});

test("latest_label_uses_newest_nonempty_line", () => {
  assert.equal(
    agentProgressLatestLabel(["▸ Working", "⏳ read router.rs"]),
    "⏳ read router.rs",
  );
});

test("active_until_done_or_stale", () => {
  assert.equal(agentProgressIsActive("⏳ read", 100, 110), true);
  assert.equal(agentProgressIsActive("✅ Done", 100, 110), false);
  assert.equal(agentProgressIsActive("⏳ read", 100, 300), false);
});
