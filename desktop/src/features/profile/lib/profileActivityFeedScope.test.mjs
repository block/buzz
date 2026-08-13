/**
 * Fold-contract tests for deriveProfileActivityFeedScope's channelActivity
 * input — the durable per-agent channel→latest-activity-ms summary that
 * repairs the profile feed's channel scope after the observer store truncates
 * an unpinned agent's event window to its newest tail.
 *
 * The five consumers of the per-agent event window are:
 *   - auto-restart (reads connectionState — a separate store field, untouched
 *     by window truncation);
 *   - working-state (reads activeAgentTurnsStore — a separate store);
 *   - latest-session (reads latestLiveSessionByAgentChannel — a separate store);
 *   - preventSleep (reads only events[last] — the newest event, which the tail
 *     always keeps);
 *   - profile-feed (reads the FULL event array for its channel set — the one
 *     consumer truncation degrades).
 * The first four are equivalent before and after truncation by construction;
 * these tests pin the fifth: with an empty summary the derived scope is
 * identical to the events/transcript-only behaviour (equivalence baseline), and
 * a non-empty summary re-adds exactly the channels and recency truncation drops.
 */

import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { deriveProfileActivityFeedScope } from "./profileActivityFeedScope.ts";

/** One observer event with a channel and an ISO timestamp derived from `ms`. */
function event(channelId, ms, seq = 0) {
  return {
    seq,
    timestamp: new Date(ms).toISOString(),
    kind: "acp_write",
    agentIndex: 0,
    channelId,
    sessionId: "s",
    turnId: "t",
    payload: {},
  };
}

describe("deriveProfileActivityFeedScope — channelActivity fold", () => {
  it("test_empty_summary_is_identical_to_events_only_baseline", () => {
    const events = [event("chan-a", 2000), event("chan-b", 3000)];
    const withoutSummary = deriveProfileActivityFeedScope({
      activeTurns: [],
      events,
      transcript: [],
    });
    const withEmptySummary = deriveProfileActivityFeedScope({
      activeTurns: [],
      events,
      transcript: [],
      channelActivity: {},
    });
    assert.deepEqual(
      withEmptySummary,
      withoutSummary,
      "an empty (or omitted) summary must not change the derived scope",
    );
    assert.deepEqual(withoutSummary.channelIds, ["chan-a", "chan-b"]);
  });

  it("test_summary_readds_channel_dropped_from_truncated_events", () => {
    // The tail kept only chan-b (newest); chan-a fell out of the event window
    // but survives in the durable summary — it must reappear in the scope.
    const truncatedEvents = [event("chan-b", 3000)];
    const scope = deriveProfileActivityFeedScope({
      activeTurns: [],
      events: truncatedEvents,
      transcript: [],
      channelActivity: { "chan-a": 2000, "chan-b": 3000 },
    });
    assert.deepEqual(
      scope.channelIds,
      ["chan-a", "chan-b"],
      "a channel dropped from the tail is re-added from the summary",
    );
    assert.equal(scope.latestActivityAtByChannel["chan-a"], 2000);
    assert.equal(scope.latestActivityAtByChannel["chan-b"], 3000);
  });

  it("test_fresher_live_event_wins_over_summary_recency", () => {
    // The summary carries a stale recency for chan-b; a fresher live event for
    // the same channel is still in the window and must win the max.
    const scope = deriveProfileActivityFeedScope({
      activeTurns: [],
      events: [event("chan-b", 9000)],
      transcript: [],
      channelActivity: { "chan-b": 3000 },
    });
    assert.equal(
      scope.latestActivityAtByChannel["chan-b"],
      9000,
      "a fresher live event beats the summary's older recency",
    );
  });

  it("test_summary_alone_gives_feed_content_and_channels", () => {
    // Every event aged out of the tail (empty window) but the summary retains
    // the channels: the feed must still report content and its channel set.
    const scope = deriveProfileActivityFeedScope({
      activeTurns: [],
      events: [],
      transcript: [],
      channelActivity: { "chan-a": 1000, "chan-c": 2000 },
    });
    assert.equal(
      scope.hasFeedContent,
      true,
      "a non-empty summary means the agent has feed content",
    );
    assert.deepEqual(scope.channelIds, ["chan-a", "chan-c"]);
  });

  it("test_live_scope_ignores_summary_for_channel_ids", () => {
    // When active turns exist the scope is live: channelIds come from the turns
    // (the agent's current work), NOT the historical summary. The summary still
    // feeds latestActivityAtByChannel so past channels keep a recency.
    const scope = deriveProfileActivityFeedScope({
      activeTurns: [{ channelId: "chan-live", anchorAt: 5000 }],
      events: [],
      transcript: [],
      channelActivity: { "chan-old": 1000 },
    });
    assert.equal(scope.isLive, true);
    assert.deepEqual(
      scope.channelIds,
      ["chan-live"],
      "live channelIds are the active turns, not the summary",
    );
    assert.equal(scope.preferredChannelId, "chan-live");
    assert.equal(
      scope.latestActivityAtByChannel["chan-old"],
      1000,
      "the summary still contributes recency for a non-live channel",
    );
  });
});
