import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  buildStableActivityStatus,
  formatElapsed,
  formatStatusSegments,
  formatTokens,
} from "./botActivityStatus.ts";

const CHANNEL = "channel-1";
const TURN = "session-100";
const OLD_TURN = "session-50";

function tool(overrides = {}) {
  return {
    id: `tool:${CHANNEL}:${Math.random()}`,
    type: "tool",
    renderClass: "shell",
    descriptor: { preview: null },
    title: "Bash",
    toolName: "shell",
    buzzToolName: "shell",
    status: "completed",
    args: {},
    result: "",
    isError: false,
    timestamp: "2026-08-03T12:00:00Z",
    startedAt: "2026-08-03T12:00:00Z",
    completedAt: null,
    turnId: TURN,
    channelId: CHANNEL,
    ...overrides,
  };
}

function thought(overrides = {}) {
  return {
    id: `thought:${Math.random()}`,
    type: "thought",
    renderClass: "thought",
    title: "Thinking",
    text: "…",
    timestamp: "2026-08-03T12:00:00Z",
    turnId: TURN,
    channelId: CHANNEL,
    ...overrides,
  };
}

function usage(text, overrides = {}) {
  return {
    id: `usage:${CHANNEL}:${TURN}`,
    type: "lifecycle",
    renderClass: "status",
    title: "Usage",
    text,
    timestamp: "2026-08-03T12:00:00Z",
    turnId: TURN,
    channelId: CHANNEL,
    ...overrides,
  };
}

describe("formatTokens", () => {
  it("keeps small counts, rounds K, trims whole M", () => {
    assert.equal(formatTokens(950), "950");
    assert.equal(formatTokens(87_500), "88K");
    assert.equal(formatTokens(118_000), "118K");
    assert.equal(formatTokens(1_000_000), "1M");
    assert.equal(formatTokens(1_500_000), "1.5M");
  });
});

describe("formatElapsed", () => {
  it("scales seconds → minutes → hours and clamps negatives", () => {
    assert.equal(formatElapsed(42_000), "42s");
    assert.equal(formatElapsed(130_000), "2m 10s");
    assert.equal(formatElapsed(3_840_000), "1h 4m");
    assert.equal(formatElapsed(-5_000), "0s");
  });
});

describe("buildStableActivityStatus", () => {
  it("prefers the newest running tool and counts turn tools", () => {
    const status = buildStableActivityStatus(
      [
        tool({ title: "Read", status: "completed" }),
        tool({
          title: "Bash",
          status: "executing",
          descriptor: { preview: "npm test" },
        }),
      ],
      CHANNEL,
    );
    assert.equal(status.activity, "Bash: npm test");
    assert.equal(status.toolCount, 2);
  });

  it("falls back to the newest phase item when no tool is running", () => {
    const status = buildStableActivityStatus(
      [tool({ status: "completed" }), thought()],
      CHANNEL,
    );
    assert.equal(status.activity, "Thinking");
  });

  it("scopes counters to the current turn and channel", () => {
    const status = buildStableActivityStatus(
      [
        tool({ turnId: OLD_TURN }),
        tool({ channelId: "channel-2", turnId: "other-session" }),
        tool({ status: "executing" }),
      ],
      CHANNEL,
    );
    assert.equal(status.toolCount, 1);
  });

  it("excludes suppressed rows from count and activity", () => {
    const status = buildStableActivityStatus(
      [tool(), tool({ renderClass: "suppressed", status: "executing" })],
      CHANNEL,
    );
    assert.equal(status.toolCount, 1);
    assert.equal(status.activity, "Bash");
  });

  it("parses the newest usage reading into a compact ctx string", () => {
    const status = buildStableActivityStatus(
      [usage("Tokens: 118000/1000000"), tool({ status: "executing" })],
      CHANNEL,
    );
    assert.equal(status.context, "118K/1M");
  });

  it("shortens absolute-path previews to their basename", () => {
    const status = buildStableActivityStatus(
      [
        tool({
          title: "Read",
          status: "executing",
          renderClass: "file-read",
          descriptor: { preview: "/Users/tolga/projects/bot/src/index.ts" },
        }),
      ],
      CHANNEL,
    );
    assert.equal(status.activity, "Read: index.ts");
  });

  it("returns the idle shape for an empty transcript", () => {
    const status = buildStableActivityStatus([], CHANNEL);
    assert.deepEqual(status, {
      activity: "Working",
      toolCount: 0,
      context: null,
    });
  });
});

describe("formatStatusSegments", () => {
  it("joins segments and omits empty counters", () => {
    assert.equal(
      formatStatusSegments(
        { activity: "Bash: npm test", toolCount: 12, context: "118K/1M" },
        "2m 10s",
      ),
      "Bash: npm test · 2m 10s · 12 tools · ctx 118K/1M",
    );
    assert.equal(
      formatStatusSegments(
        { activity: "Working", toolCount: 0, context: null },
        null,
      ),
      "Working",
    );
    assert.equal(
      formatStatusSegments(
        { activity: "Thinking", toolCount: 1, context: null },
        "5s",
      ),
      "Thinking · 5s · 1 tool",
    );
  });
});

describe("buildStableActivityStatus previews", () => {
  it("prefers a human description over the raw command preview", () => {
    const status = buildStableActivityStatus(
      [
        tool({
          status: "executing",
          args: {
            command: "sed -n '420,432p' file.ts",
            description: "Read the store cache",
          },
          descriptor: { preview: "sed -n '420,432p' file.ts" },
        }),
      ],
      CHANNEL,
    );
    assert.equal(status.activity, "Bash: Read the store cache");
  });
});

describe("buildStableActivityStatus thread scoping", () => {
  it("locks onto the thread's turn when threadRootId is given", () => {
    const status = buildStableActivityStatus(
      [
        tool({
          sessionId: "aaaa000011112222",
          turnId: "t-a",
          status: "executing",
          descriptor: { preview: "thread-a-work" },
        }),
        tool({
          sessionId: "bbbb000011112222",
          turnId: "t-b",
          status: "executing",
          descriptor: { preview: "thread-b-work" },
        }),
      ],
      CHANNEL,
      "aaaa000011112222deadbeefdeadbeef",
    );
    assert.equal(status.activity, "Bash: thread-a-work");
    assert.equal(status.toolCount, 1);
  });
});
