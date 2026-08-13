/**
 * Store-level eviction tests for the channel-scoped observer archive.
 *
 * These exercise the REAL ingest path (ingestArchivedObserverEvents with an
 * injected decrypt fn) plus the public pin API, asserting the LRU bound and its
 * invariants at the store boundary — the layer above the pure policy unit:
 *
 *   - the retained-channel cap is enforced across ingest pages;
 *   - eviction is least-recently-loaded first;
 *   - a pinned channel is never evicted (even when least recent);
 *   - evicting a channel drops every (agent, channel) entry it owns;
 *   - an evicted channel reads back empty (cache miss → re-hydrate on revisit);
 *   - resetAgentObserverStore clears the LRU bookkeeping so counts don't leak.
 *
 * No React/Tauri runtime: security guards are satisfied via _testRegisterKnownAgents
 * and decrypt is injected. The cap (MAX_RETAINED_ARCHIVE_CHANNELS = 8) is a
 * private constant; tests drive enough distinct channels to cross it and assert
 * behaviour relative to that threshold rather than importing the value.
 */

import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";

import {
  ingestArchivedObserverEvents,
  resetAgentObserverStore,
  pinArchiveChannel,
  unpinArchiveChannel,
  _testRegisterKnownAgents,
  _testGetArchivedChannelEvents,
} from "@/features/agents/observerRelayStore.ts";

const CAP = 8; // MAX_RETAINED_ARCHIVE_CHANNELS (private constant, mirrored here)
const SUB_ID = "test-eviction-sub";

/** 64-hex agent pubkey from a short label. */
function agentPubkey(label) {
  return label.padEnd(64, "0");
}

const AGENT_A = agentPubkey("a");
const AGENT_B = agentPubkey("b");

/** Raw kind-24200 telemetry row for (agent, channel); content carries the event. */
function makeRawEvent(agent, channelId, seq) {
  return {
    id: `${seq}`.padStart(64, "e"),
    pubkey: agent,
    created_at: 1000 + seq,
    kind: 24200,
    tags: [
      ["p", "d".repeat(64)],
      ["agent", agent],
      ["frame", "telemetry"],
    ],
    content: JSON.stringify({
      seq,
      timestamp: new Date(1_000_000 + seq * 1000).toISOString(),
      kind: "acp_write",
      agentIndex: 0,
      channelId,
      sessionId: "sess-1",
      turnId: "turn-1",
      payload: {},
    }),
    sig: "s".repeat(128),
  };
}

/** Decrypt fn: content is the JSON-encoded ObserverEvent. */
const decrypt = (event) => Promise.resolve(JSON.parse(event.content));

/** Ingest one telemetry frame for (agent, channel), awaiting the store write. */
async function ingestOne(agent, channelId, seq) {
  await ingestArchivedObserverEvents(
    [makeRawEvent(agent, channelId, seq)],
    decrypt,
  );
}

describe("observer archive eviction — store level", () => {
  beforeEach(() => {
    resetAgentObserverStore();
    _testRegisterKnownAgents(SUB_ID, [AGENT_A, AGENT_B]);
  });

  it("test_channels_up_to_cap_are_all_retained", async () => {
    for (let i = 0; i < CAP; i++) {
      await ingestOne(AGENT_A, `chan-${i}`, i);
    }
    for (let i = 0; i < CAP; i++) {
      assert.equal(
        _testGetArchivedChannelEvents(AGENT_A, `chan-${i}`).length,
        1,
        `chan-${i} must be retained at exactly cap`,
      );
    }
  });

  it("test_exceeding_cap_evicts_least_recently_loaded_channel", async () => {
    // Load CAP+1 distinct channels in ascending order. chan-0 is the oldest
    // load and unpinned, so it is the one evicted when the cap is crossed.
    for (let i = 0; i <= CAP; i++) {
      await ingestOne(AGENT_A, `chan-${i}`, i);
    }
    assert.equal(
      _testGetArchivedChannelEvents(AGENT_A, "chan-0").length,
      0,
      "least-recently-loaded channel must be evicted",
    );
    // The newest CAP channels survive.
    for (let i = 1; i <= CAP; i++) {
      assert.equal(
        _testGetArchivedChannelEvents(AGENT_A, `chan-${i}`).length,
        1,
        `chan-${i} must survive`,
      );
    }
  });

  it("test_pinned_channel_survives_even_when_least_recently_loaded", async () => {
    // chan-0 is loaded first (oldest) but pinned; loading CAP more channels
    // crosses the cap. The eviction must fall on the oldest UNPINNED channel
    // (chan-1), never the pinned chan-0.
    await ingestOne(AGENT_A, "chan-0", 0);
    pinArchiveChannel("chan-0");
    for (let i = 1; i <= CAP; i++) {
      await ingestOne(AGENT_A, `chan-${i}`, i);
    }
    assert.equal(
      _testGetArchivedChannelEvents(AGENT_A, "chan-0").length,
      1,
      "pinned channel must never be evicted",
    );
    assert.equal(
      _testGetArchivedChannelEvents(AGENT_A, "chan-1").length,
      0,
      "oldest unpinned channel must be evicted instead of the pinned one",
    );
  });

  it("test_unpinning_makes_channel_evictable_again", async () => {
    // Pin protects chan-0 across a first over-cap wave; after unpinning, a
    // second wave that again exceeds the cap must be free to evict chan-0.
    await ingestOne(AGENT_A, "chan-0", 0);
    pinArchiveChannel("chan-0");
    for (let i = 1; i <= CAP; i++) {
      await ingestOne(AGENT_A, `chan-${i}`, i);
    }
    assert.equal(
      _testGetArchivedChannelEvents(AGENT_A, "chan-0").length,
      1,
      "pinned channel survives the first wave",
    );

    unpinArchiveChannel("chan-0");
    // chan-0 is now the least-recently-loaded unpinned channel. Load fresh
    // channels to push the retained set past the cap again.
    for (let i = CAP + 1; i <= CAP * 2; i++) {
      await ingestOne(AGENT_A, `chan-${i}`, i);
    }
    assert.equal(
      _testGetArchivedChannelEvents(AGENT_A, "chan-0").length,
      0,
      "unpinned channel becomes evictable and is dropped",
    );
  });

  it("test_refcounted_pin_holds_until_last_release", async () => {
    // Two mounted consumers pin chan-0; one unpins. The channel must remain
    // protected until the SECOND unpin — refcount, not a boolean flag.
    await ingestOne(AGENT_A, "chan-0", 0);
    pinArchiveChannel("chan-0");
    pinArchiveChannel("chan-0");
    unpinArchiveChannel("chan-0");

    for (let i = 1; i <= CAP; i++) {
      await ingestOne(AGENT_A, `chan-${i}`, i);
    }
    assert.equal(
      _testGetArchivedChannelEvents(AGENT_A, "chan-0").length,
      1,
      "channel stays pinned while one consumer still holds a pin",
    );

    unpinArchiveChannel("chan-0");
    for (let i = CAP + 1; i <= CAP * 2; i++) {
      await ingestOne(AGENT_A, `chan-${i}`, i);
    }
    assert.equal(
      _testGetArchivedChannelEvents(AGENT_A, "chan-0").length,
      0,
      "channel is evictable only after the last pin is released",
    );
  });

  it("test_eviction_drops_all_agent_entries_for_the_channel", async () => {
    // chan-shared is populated for two agents (fan-out: one channel view
    // populates multiple (agent, channel) keys). Both keys must be evicted
    // together when the channel is dropped.
    await ingestOne(AGENT_A, "chan-shared", 0);
    await ingestOne(AGENT_B, "chan-shared", 1);
    // Push past the cap with unrelated channels; chan-shared is oldest.
    for (let i = 0; i < CAP; i++) {
      await ingestOne(AGENT_A, `chan-other-${i}`, 100 + i);
    }
    assert.equal(
      _testGetArchivedChannelEvents(AGENT_A, "chan-shared").length,
      0,
      "agent-A entry for the evicted channel must be gone",
    );
    assert.equal(
      _testGetArchivedChannelEvents(AGENT_B, "chan-shared").length,
      0,
      "agent-B entry for the same evicted channel must be gone too",
    );
  });

  it("test_evicted_channel_reads_back_empty_for_rehydration", async () => {
    // The revisit contract: an evicted channel returns an empty window, which
    // the paging hook treats as a cache miss and re-hydrates from SQLite. Here
    // we assert the empty read; re-ingesting the same page restores it.
    for (let i = 0; i <= CAP; i++) {
      await ingestOne(AGENT_A, `chan-${i}`, i);
    }
    assert.equal(
      _testGetArchivedChannelEvents(AGENT_A, "chan-0").length,
      0,
      "evicted channel reads back empty",
    );
    // Re-hydrate: re-ingesting restores the window (proves eviction is not a
    // permanent tombstone — the key is fully reusable).
    await ingestOne(AGENT_A, "chan-0", 0);
    assert.equal(
      _testGetArchivedChannelEvents(AGENT_A, "chan-0").length,
      1,
      "re-ingest after eviction restores the channel window",
    );
  });

  it("test_reset_clears_pin_state_so_counts_do_not_leak", async () => {
    // A pin outstanding at reset must not survive it — otherwise a phantom pin
    // would protect a channel id in the next session. After reset we re-pin
    // NOTHING and confirm the same channel id is freely evictable.
    await ingestOne(AGENT_A, "chan-0", 0);
    pinArchiveChannel("chan-0");
    resetAgentObserverStore();
    _testRegisterKnownAgents(SUB_ID, [AGENT_A, AGENT_B]);

    // If reset leaked the pin, chan-0 would survive this over-cap wave.
    for (let i = 0; i <= CAP; i++) {
      await ingestOne(AGENT_A, `chan-${i}`, i);
    }
    assert.equal(
      _testGetArchivedChannelEvents(AGENT_A, "chan-0").length,
      0,
      "pin state must not survive reset — chan-0 is freely evictable again",
    );
  });
});
