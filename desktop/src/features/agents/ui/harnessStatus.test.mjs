import assert from "node:assert/strict";
import test from "node:test";

import {
  buzzwordAt,
  deriveHarnessStatus,
  formatElapsed,
  formatTokens,
  HARNESS_BUZZWORDS,
  parseUsageText,
  shellCommandOf,
} from "./harnessStatus.ts";

test("buzzwords cycle and never index out of range", () => {
  assert.equal(buzzwordAt(0), HARNESS_BUZZWORDS[0]);
  assert.equal(buzzwordAt(HARNESS_BUZZWORDS.length), HARNESS_BUZZWORDS[0]);
  assert.equal(buzzwordAt(-1), HARNESS_BUZZWORDS[HARNESS_BUZZWORDS.length - 1]);
});

test("parses tokens and cost out of a usage row", () => {
  assert.deepEqual(parseUsageText("Tokens: 32048/1000000 ($0.1754 USD)"), {
    used: 32048,
    size: 1000000,
    cost: "$0.1754",
  });
});

test("parses tokens when the provider reports no cost", () => {
  assert.deepEqual(parseUsageText("Tokens: 500/1000"), {
    used: 500,
    size: 1000,
    cost: null,
  });
});

test("usage parsing tolerates missing or junk text", () => {
  assert.deepEqual(parseUsageText(null), {
    used: null,
    size: null,
    cost: null,
  });
  assert.deepEqual(parseUsageText("no numbers here"), {
    used: null,
    size: null,
    cost: null,
  });
});

test("extracts a shell command from any known arg key", () => {
  assert.equal(
    shellCommandOf({
      id: "t",
      type: "tool",
      args: { command: " ls -la " },
      timestamp: "",
    }),
    "ls -la",
  );
  assert.equal(
    shellCommandOf({
      id: "t",
      type: "tool",
      args: { cmd: "pwd" },
      timestamp: "",
    }),
    "pwd",
  );
  assert.equal(
    shellCommandOf({ id: "t", type: "tool", args: {}, timestamp: "" }),
    null,
  );
  assert.equal(
    shellCommandOf({
      id: "t",
      type: "tool",
      args: { command: "   " },
      timestamp: "",
    }),
    null,
  );
});

test("derives running commands and tool progress", () => {
  const status = deriveHarnessStatus([
    {
      id: "tool:1",
      type: "tool",
      status: "completed",
      args: { command: "ls" },
      timestamp: "1",
    },
    {
      id: "tool:2",
      type: "tool",
      status: "executing",
      args: { command: "cd /tmp" },
      timestamp: "2",
    },
    {
      id: "usage:c:t",
      type: "lifecycle",
      text: "Tokens: 3200/100000 ($0.02 USD)",
      timestamp: "3",
    },
    {
      id: "thinking:c:t",
      type: "thought",
      text: "Checking the call sites\nmore detail",
      timestamp: "4",
    },
  ]);
  assert.deepEqual(status.runningCommands, ["cd /tmp"]);
  assert.equal(status.toolsTotal, 2);
  assert.equal(status.toolsDone, 1);
  assert.equal(status.tokensUsed, 3200);
  assert.equal(status.cost, "$0.02");
  assert.equal(status.summary, "Checking the call sites");
});

test("uses the newest usage row when several exist", () => {
  const status = deriveHarnessStatus([
    {
      id: "usage:c:t",
      type: "lifecycle",
      text: "Tokens: 100/1000",
      timestamp: "1",
    },
    {
      id: "usage:c:t2",
      type: "lifecycle",
      text: "Tokens: 900/1000",
      timestamp: "2",
    },
  ]);
  assert.equal(status.tokensUsed, 900);
});

test("returns an empty status for an empty transcript", () => {
  const status = deriveHarnessStatus([]);
  assert.deepEqual(status.runningCommands, []);
  assert.equal(status.toolsTotal, 0);
  assert.equal(status.tokensUsed, null);
  assert.equal(status.summary, null);
});

test("formats elapsed time like Claude Code", () => {
  assert.equal(formatElapsed(3200), "3s");
  assert.equal(formatElapsed(77000), "1m 17s");
  assert.equal(formatElapsed(0), "0s");
  assert.equal(formatElapsed(-50), "0s");
});

test("formats token counts compactly", () => {
  assert.equal(formatTokens(980), "980");
  assert.equal(formatTokens(3200), "3.2k");
});
