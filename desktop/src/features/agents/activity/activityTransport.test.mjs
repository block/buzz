import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  activityTransportBadge,
  createActivityTransport,
  cycleActivitySpeed,
  jumpActivityTransport,
  jumpToLiveActivityTransport,
  LIVE_REJOIN_THRESHOLD_MS,
  restartActivityTransport,
  scrubActivityTransport,
  tickActivityTransport,
  toggleActivityPlayback,
} from "./activityTransport.ts";

const LIVE_BOUNDS = { d0: 0, d1: 60_000, isLive: true };
const ENDED_BOUNDS = { d0: 0, d1: 60_000, isLive: false };

describe("createActivityTransport", () => {
  it("starts live sessions in live mode at the edge", () => {
    const state = createActivityTransport(LIVE_BOUNDS);
    assert.equal(state.mode, "live");
    assert.equal(state.playing, true);
    assert.equal(state.viewT, 60_000);
  });

  it("starts idle scopes parked at the end", () => {
    const state = createActivityTransport(ENDED_BOUNDS);
    assert.equal(state.mode, "replay");
    assert.equal(state.playing, false);
    assert.equal(state.viewT, 60_000);
  });
});

describe("tickActivityTransport", () => {
  it("pins the playhead to the growing live edge", () => {
    const state = createActivityTransport(LIVE_BOUNDS);
    const grown = { ...LIVE_BOUNDS, d1: 61_000 };
    const next = tickActivityTransport(state, 16, grown);
    assert.equal(next.viewT, 61_000);
    assert.equal(next.mode, "live");
  });

  it("parks at the end when a live session ends", () => {
    const state = createActivityTransport(LIVE_BOUNDS);
    const next = tickActivityTransport(state, 16, ENDED_BOUNDS);
    assert.equal(next.mode, "replay");
    assert.equal(next.playing, false);
    assert.equal(next.viewT, 60_000);
  });

  it("advances replay by dt times speed", () => {
    let state = scrubActivityTransport(
      createActivityTransport(ENDED_BOUNDS),
      10_000,
      ENDED_BOUNDS,
    );
    state = { ...state, playing: true, speed: 4 };
    const next = tickActivityTransport(state, 100, ENDED_BOUNDS);
    assert.equal(next.viewT, 10_400);
  });

  it("rejoins live when replay catches the edge", () => {
    const nearEdge = LIVE_BOUNDS.d1 - LIVE_REJOIN_THRESHOLD_MS + 1;
    let state = scrubActivityTransport(
      createActivityTransport(LIVE_BOUNDS),
      nearEdge - 10,
      LIVE_BOUNDS,
    );
    state = { ...state, playing: true };
    const next = tickActivityTransport(state, 20, LIVE_BOUNDS);
    assert.equal(next.mode, "live");
    assert.equal(next.viewT, LIVE_BOUNDS.d1);
  });

  it("stops at the end of an idle scope without rejoining", () => {
    let state = scrubActivityTransport(
      createActivityTransport(ENDED_BOUNDS),
      59_990,
      ENDED_BOUNDS,
    );
    state = { ...state, playing: true };
    const next = tickActivityTransport(state, 100, ENDED_BOUNDS);
    assert.equal(next.mode, "replay");
    assert.equal(next.playing, false);
    assert.equal(next.viewT, 60_000);
  });

  it("does not move while paused", () => {
    const state = {
      mode: "replay",
      playing: false,
      speed: 1,
      viewT: 30_000,
    };
    const next = tickActivityTransport(state, 100, ENDED_BOUNDS);
    assert.equal(next.viewT, 30_000);
  });
});

describe("scrub and jump", () => {
  it("scrubbing drops live mode into replay and clamps to bounds", () => {
    const state = createActivityTransport(LIVE_BOUNDS);
    const scrubbed = scrubActivityTransport(state, -5_000, LIVE_BOUNDS);
    assert.equal(scrubbed.mode, "replay");
    assert.equal(scrubbed.viewT, 0);
    const over = scrubActivityTransport(state, 99_000, LIVE_BOUNDS);
    assert.equal(over.viewT, 60_000);
  });

  it("jumping back from live starts replay behind the edge", () => {
    const state = createActivityTransport(LIVE_BOUNDS);
    const next = jumpActivityTransport(state, -2_000, LIVE_BOUNDS);
    assert.equal(next.mode, "replay");
    assert.equal(next.viewT, 58_000);
  });

  it("jumping forward past the edge rejoins live", () => {
    let state = scrubActivityTransport(
      createActivityTransport(LIVE_BOUNDS),
      59_500,
      LIVE_BOUNDS,
    );
    state = jumpActivityTransport(state, 2_000, LIVE_BOUNDS);
    assert.equal(state.mode, "live");
    assert.equal(state.viewT, 60_000);
  });

  it("jumping forward past the end of an idle scope parks at the end", () => {
    let state = scrubActivityTransport(
      createActivityTransport(ENDED_BOUNDS),
      59_500,
      ENDED_BOUNDS,
    );
    state = jumpActivityTransport(state, 2_000, ENDED_BOUNDS);
    assert.equal(state.mode, "replay");
    assert.equal(state.viewT, 60_000);
  });
});

describe("toggleActivityPlayback", () => {
  it("pausing live freezes the playhead in replay", () => {
    const state = createActivityTransport(LIVE_BOUNDS);
    const next = toggleActivityPlayback(state, LIVE_BOUNDS);
    assert.equal(next.mode, "replay");
    assert.equal(next.playing, false);
    assert.equal(next.viewT, 60_000);
  });

  it("resuming at the end of an idle scope restarts from the top", () => {
    const state = createActivityTransport(ENDED_BOUNDS);
    const next = toggleActivityPlayback(state, ENDED_BOUNDS);
    assert.equal(next.playing, true);
    assert.equal(next.viewT, 0);
  });

  it("plain pause/resume mid-replay keeps the playhead", () => {
    let state = scrubActivityTransport(
      createActivityTransport(ENDED_BOUNDS),
      20_000,
      ENDED_BOUNDS,
    );
    state = { ...state, playing: true };
    const paused = toggleActivityPlayback(state, ENDED_BOUNDS);
    assert.equal(paused.playing, false);
    assert.equal(paused.viewT, 20_000);
    const resumed = toggleActivityPlayback(paused, ENDED_BOUNDS);
    assert.equal(resumed.playing, true);
    assert.equal(resumed.viewT, 20_000);
  });
});

describe("live jump, restart, speed, badge", () => {
  it("jumpToLive returns to the edge only while live", () => {
    let state = scrubActivityTransport(
      createActivityTransport(LIVE_BOUNDS),
      10_000,
      LIVE_BOUNDS,
    );
    state = jumpToLiveActivityTransport(state, LIVE_BOUNDS);
    assert.equal(state.mode, "live");

    let ended = scrubActivityTransport(
      createActivityTransport(ENDED_BOUNDS),
      10_000,
      ENDED_BOUNDS,
    );
    ended = jumpToLiveActivityTransport(ended, ENDED_BOUNDS);
    assert.equal(ended.mode, "replay");
    assert.equal(ended.viewT, 10_000);
  });

  it("restart plays from the beginning", () => {
    const state = createActivityTransport(ENDED_BOUNDS);
    const next = restartActivityTransport(state, ENDED_BOUNDS);
    assert.equal(next.playing, true);
    assert.equal(next.viewT, 0);
    assert.equal(next.mode, "replay");
  });

  it("cycles speeds 1 -> 2 -> 4 -> 8 -> 1", () => {
    let state = createActivityTransport(ENDED_BOUNDS);
    const seen = [];
    for (let index = 0; index < 4; index += 1) {
      state = cycleActivitySpeed(state);
      seen.push(state.speed);
    }
    assert.deepEqual(seen, [2, 4, 8, 1]);
  });

  it("reports live, replay, and idle badges", () => {
    const live = createActivityTransport(LIVE_BOUNDS);
    assert.equal(activityTransportBadge(live, LIVE_BOUNDS), "live");

    const replay = scrubActivityTransport(live, 10_000, LIVE_BOUNDS);
    assert.equal(activityTransportBadge(replay, LIVE_BOUNDS), "replay");

    // Parked at the end of an inactive scope: idle (it can wake again),
    // never "ended" — the scope's future is not the transport's to claim.
    const idle = createActivityTransport(ENDED_BOUNDS);
    assert.equal(activityTransportBadge(idle, ENDED_BOUNDS), "idle");
  });
});
