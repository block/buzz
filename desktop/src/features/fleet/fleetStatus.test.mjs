import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  deriveActivityHeadline,
  deriveFleetLiveStatus,
  deriveLastSeenMs,
  formatLastSeen,
  formatTokenCount,
  formatUsdCost,
} from "./fleetStatus.ts";

describe("deriveFleetLiveStatus", () => {
  it("working wins over everything else", () => {
    assert.equal(
      deriveFleetLiveStatus({
        working: true,
        status: "stopped",
        lastError: "boom",
      }),
      "working",
    );
  });

  it("running/deployed without work is idle", () => {
    assert.equal(
      deriveFleetLiveStatus({
        working: false,
        status: "running",
        lastError: null,
      }),
      "idle",
    );
    assert.equal(
      deriveFleetLiveStatus({
        working: false,
        status: "deployed",
        // A stale error from a previous run must not shadow a live process.
        lastError: "old error",
      }),
      "idle",
    );
  });

  it("down with a recorded error needs attention", () => {
    assert.equal(
      deriveFleetLiveStatus({
        working: false,
        status: "stopped",
        lastError: "exit 1",
      }),
      "error",
    );
  });

  it("down without an error is offline", () => {
    assert.equal(
      deriveFleetLiveStatus({
        working: false,
        status: "stopped",
        lastError: null,
      }),
      "offline",
    );
    assert.equal(
      deriveFleetLiveStatus({
        working: false,
        status: "not_deployed",
        lastError: null,
      }),
      "offline",
    );
  });
});

describe("deriveLastSeenMs", () => {
  const event = (timestamp) => ({
    seq: 1,
    timestamp,
    kind: "turn_liveness",
    agentIndex: 0,
    channelId: null,
    sessionId: null,
    turnId: null,
    payload: null,
  });

  it("uses the newest source among observer frames and lifecycle stamps", () => {
    const result = deriveLastSeenMs({
      events: [event("2026-07-29T10:00:00Z"), event("2026-07-29T12:00:00Z")],
      lastStartedAt: "2026-07-29T09:00:00Z",
      lastStoppedAt: "2026-07-29T11:00:00Z",
    });
    assert.equal(result, Date.parse("2026-07-29T12:00:00Z"));
  });

  it("falls back to lifecycle stamps without observer frames", () => {
    const result = deriveLastSeenMs({
      events: [],
      lastStartedAt: null,
      lastStoppedAt: "2026-07-29T11:00:00Z",
    });
    assert.equal(result, Date.parse("2026-07-29T11:00:00Z"));
  });

  it("returns null with no parseable source", () => {
    assert.equal(
      deriveLastSeenMs({ events: [event("not-a-date")], lastStartedAt: null }),
      null,
    );
    assert.equal(deriveLastSeenMs({ events: [] }), null);
  });
});

describe("deriveActivityHeadline", () => {
  const tool = (title, renderClass = "shell") => ({
    id: `tool-${title}`,
    type: "tool",
    renderClass,
    descriptor: { renderClass, label: title, preview: null },
    title,
    toolName: "shell",
    buzzToolName: null,
    status: "completed",
    args: {},
    result: "",
    isError: false,
    timestamp: "2026-07-29T12:00:00Z",
    startedAt: "2026-07-29T12:00:00Z",
    completedAt: null,
  });

  it("returns the newest meaningful item's title", () => {
    assert.equal(
      deriveActivityHeadline([tool("Read src/a.ts"), tool("Ran cargo test")]),
      "Ran cargo test",
    );
  });

  it("uses message text rather than the speaker-role title", () => {
    const message = {
      id: "m1",
      type: "message",
      renderClass: "message",
      role: "assistant",
      title: "Assistant",
      text: "Deployed the fix to staging.",
      timestamp: "2026-07-29T12:00:00Z",
    };
    assert.equal(
      deriveActivityHeadline([tool("Older"), message]),
      "Deployed the fix to staging.",
    );
  });

  it("skips suppressed and raw-rail items", () => {
    const suppressed = { ...tool("hidden"), renderClass: "suppressed" };
    assert.equal(
      deriveActivityHeadline([tool("Visible"), suppressed]),
      "Visible",
    );
  });

  it("returns null for an empty transcript", () => {
    assert.equal(deriveActivityHeadline([]), null);
  });
});

describe("formatting", () => {
  it("formats token counts compactly", () => {
    assert.equal(formatTokenCount(950), "950");
    assert.equal(formatTokenCount(12_340), "12.3k");
    assert.equal(formatTokenCount(2_000), "2k");
    assert.equal(formatTokenCount(4_200_000), "4.2M");
  });

  it("formats costs with a sub-cent floor", () => {
    assert.equal(formatUsdCost(0), "$0.00");
    assert.equal(formatUsdCost(0.004), "<$0.01");
    assert.equal(formatUsdCost(1.5), "$1.50");
  });

  it("formats last-seen labels across tiers", () => {
    const now = Date.parse("2026-07-29T12:00:00Z");
    assert.equal(formatLastSeen(now - 20_000, now), "just now");
    assert.equal(formatLastSeen(now - 5 * 60_000, now), "5m ago");
    assert.equal(formatLastSeen(now - 3 * 3_600_000, now), "3h ago");
    assert.equal(formatLastSeen(now - 2 * 86_400_000, now), "2d ago");
    // A skewed future timestamp clamps instead of going negative.
    assert.equal(formatLastSeen(now + 60_000, now), "just now");
  });
});
