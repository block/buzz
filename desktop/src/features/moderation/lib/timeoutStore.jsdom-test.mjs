/**
 * jsdom behavior test for the timeout store's clear-at-expiry effect.
 *
 * The pure-node timeoutStore.test.mjs covers record/clear/snapshot semantics,
 * but the reactive clear-at-expiry effect in `useTimeoutState`
 * (timeoutStore.ts: `if (!derived.active && state.active) clearTimeoutState()`)
 * needs a rendering context to exercise. Without this test, deleting that
 * effect fails nothing — a live timeout with a known expiry would tick past its
 * deadline yet leave the store `active`, permanently blocking the composer.
 *
 * ComposerTimeoutBanner owns that hook (it mounts only while a timeout is
 * active), so mounting it here drives the production seam. We record a future
 * expiry, mount the banner, advance a controlled clock past the deadline, fire
 * the store's interval tick, and assert the store auto-clears to inactive.
 *
 * Falsifiability: deleting the clear-at-expiry effect from timeoutStore.ts
 * turns "expiry-clears-store" RED (the snapshot stays `active: true`).
 */

import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import React from "react";
import { createRoot } from "react-dom/client";
import { act } from "react";

import { ComposerTimeoutBanner } from "@/features/moderation/ui/ComposerTimeoutBanner.tsx";
import {
  recordTimeoutFromRejection,
  clearTimeoutState,
  getTimeoutSnapshot,
} from "./timeoutStore.ts";

// ── Controlled clock + interval ───────────────────────────────────────────────
// The store reads Date.now() and drives its countdown with window.setInterval.
// jsdom's window owns its own setInterval, so node:test's timer mocks don't
// reach it — we stub both here so a test can advance time and fire the tick
// deterministically inside act(), without waiting on real wall-clock seconds.

const START_MS = 1_700_000_000_000;
let currentNowMs = START_MS;
const intervalCallbacks = new Map();
let nextIntervalId = 1;

const realDateNow = Date.now;
const realSetInterval = globalThis.window.setInterval;
const realClearInterval = globalThis.window.clearInterval;

function installClock() {
  currentNowMs = START_MS;
  intervalCallbacks.clear();
  Date.now = () => currentNowMs;
  globalThis.window.setInterval = (cb) => {
    const id = nextIntervalId++;
    intervalCallbacks.set(id, cb);
    return id;
  };
  globalThis.window.clearInterval = (id) => {
    intervalCallbacks.delete(id);
  };
}

function restoreClock() {
  Date.now = realDateNow;
  globalThis.window.setInterval = realSetInterval;
  globalThis.window.clearInterval = realClearInterval;
}

/** Advance the controlled clock and fire every registered store interval. */
function advanceTo(ms) {
  currentNowMs = ms;
  for (const cb of [...intervalCallbacks.values()]) {
    cb();
  }
}

afterEach(() => {
  restoreClock();
  clearTimeoutState();
});

function mountBanner() {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  return {
    render: async () => {
      await act(async () => {
        root.render(React.createElement(ComposerTimeoutBanner));
      });
    },
    unmount: async () => {
      await act(async () => {
        root.unmount();
      });
      document.body.removeChild(container);
    },
  };
}

test("expiry-clears-store: a live known-expiry timeout auto-clears once the deadline passes", async () => {
  installClock();

  // Record a timeout expiring 60s out and confirm the store is active.
  const expiryUnixSec = Math.floor((START_MS + 60_000) / 1000);
  recordTimeoutFromRejection(
    `restricted: you are timed out until ${expiryUnixSec}`,
  );
  assert.equal(
    getTimeoutSnapshot().active,
    true,
    "store must be active after recording a future-expiry timeout",
  );

  const banner = mountBanner();
  try {
    await banner.render();
    // Still active mid-window: the countdown is live, nothing has cleared.
    assert.equal(
      getTimeoutSnapshot().active,
      true,
      "store must stay active while the timeout is still in the future",
    );

    // Push the clock past expiry and fire the store's per-second tick.
    await act(async () => {
      advanceTo(START_MS + 61_000);
    });

    // The clear-at-expiry effect must have collapsed the store to inactive.
    assert.equal(
      getTimeoutSnapshot().active,
      false,
      "store must auto-clear to inactive once the known expiry passes",
    );
    assert.equal(
      getTimeoutSnapshot().expiresAtMs,
      null,
      "expiresAtMs must be null after the auto-clear",
    );
  } finally {
    await banner.unmount();
  }
});

test("null-expiry-stays-active: an unknown-expiry timeout is not auto-cleared by ticking", async () => {
  installClock();

  // No parseable timestamp → active with null expiry, blocks until a send clears.
  recordTimeoutFromRejection("restricted: you are timed out until whenever");
  assert.equal(getTimeoutSnapshot().active, true, "active with unknown expiry");
  assert.equal(getTimeoutSnapshot().expiresAtMs, null, "expiry is null");

  const banner = mountBanner();
  try {
    await banner.render();
    // Advancing the clock cannot clear an unknown-expiry block (no interval is
    // even started); the store must remain active.
    await act(async () => {
      advanceTo(START_MS + 3_600_000);
    });
    assert.equal(
      getTimeoutSnapshot().active,
      true,
      "unknown-expiry timeout must remain active — only a successful send clears it",
    );
  } finally {
    await banner.unmount();
  }
});
