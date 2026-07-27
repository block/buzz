import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_CHANNEL_NOTIFY_STATE,
  nextTimedMuteExpiry,
  resolveChannelNotifyState,
} from "./resolveChannelNotifyState.ts";

const NOW = 1_000;

function prefs(entry) {
  return { version: 1, channels: entry ? { c: entry } : {} };
}

function legacy(entry) {
  return { version: 1, channels: entry ? { c: entry } : {} };
}

function resolve(prefsEntry, legacyEntry, now = NOW) {
  return resolveChannelNotifyState(
    "c",
    prefs(prefsEntry),
    legacy(legacyEntry),
    now,
  );
}

// ── defaults ──────────────────────────────────────────────────────────────────

test("no entry in either store returns the shared default state", () => {
  assert.equal(resolve(null, null), DEFAULT_CHANNEL_NOTIFY_STATE);
});

test("an entry with only advanced fields keeps level 'all'", () => {
  assert.deepEqual(
    resolve({
      desktop: false,
      followAllThreads: true,
      broadcasts: false,
      updatedAt: 1,
    }),
    {
      level: "all",
      timedMuteActive: false,
      desktop: false,
      followAllThreads: true,
      broadcasts: false,
      hidden: false,
    },
  );
});

// ── levels ────────────────────────────────────────────────────────────────────

test("level 'mentions' resolves without hiding", () => {
  const state = resolve({ level: "mentions", updatedAt: 1 });
  assert.equal(state.level, "mentions");
  assert.equal(state.hidden, false);
  assert.equal(state.timedMuteActive, false);
});

test("explicit level 'mute' hides the channel", () => {
  const state = resolve({ level: "mute", updatedAt: 1 });
  assert.equal(state.level, "mute");
  assert.equal(state.hidden, true);
});

test("advanced defaults apply when fields are absent", () => {
  const state = resolve({ level: "mentions", updatedAt: 1 });
  assert.equal(state.desktop, true);
  assert.equal(state.followAllThreads, false);
  assert.equal(state.broadcasts, true);
});

// ── timed mute overlay ────────────────────────────────────────────────────────

test("a running muteUntil forces level 'mute' without hiding", () => {
  const state = resolve({ muteUntil: NOW + 60, updatedAt: 1 });
  assert.equal(state.level, "mute");
  assert.equal(state.timedMuteActive, true);
  assert.equal(state.hidden, false);
});

test("timed mute overlays 'mentions' and restores it on expiry", () => {
  const entry = { level: "mentions", muteUntil: NOW + 60, updatedAt: 1 };
  assert.equal(resolve(entry).level, "mute");
  const expired = resolve(entry, null, NOW + 61);
  assert.equal(expired.level, "mentions");
  assert.equal(expired.timedMuteActive, false);
});

test("muteUntil exactly at now is already expired", () => {
  const state = resolve({ muteUntil: NOW, updatedAt: 1 });
  assert.equal(state.level, "all");
  assert.equal(state.timedMuteActive, false);
});

test("timed mute on an explicitly muted channel keeps hidden true", () => {
  const state = resolve({ level: "mute", muteUntil: NOW + 60, updatedAt: 1 });
  assert.equal(state.level, "mute");
  assert.equal(state.timedMuteActive, true);
  assert.equal(state.hidden, true);
});

// ── legacy channel-mutes interop ──────────────────────────────────────────────

test("legacy-only mute resolves to level 'mute' but never hides", () => {
  const state = resolve(null, { muted: true, updatedAt: 50 });
  assert.equal(state.level, "mute");
  assert.equal(state.hidden, false);
});

test("legacy-only unmute leaves the defaults untouched", () => {
  const state = resolve(null, { muted: false, updatedAt: 50 });
  assert.equal(state.level, "all");
  assert.equal(state.hidden, false);
});

test("newer legacy unmute beats a stale prefs 'mute'", () => {
  const state = resolve(
    { level: "mute", updatedAt: 10 },
    { muted: false, updatedAt: 20 },
  );
  assert.equal(state.level, "all");
  assert.equal(state.hidden, false);
});

test("newer legacy mute overrides prefs 'mentions' without hiding", () => {
  const state = resolve(
    { level: "mentions", updatedAt: 10 },
    { muted: true, updatedAt: 20 },
  );
  assert.equal(state.level, "mute");
  assert.equal(state.hidden, false);
});

test("newer prefs 'mute' beats a stale legacy unmute", () => {
  const state = resolve(
    { level: "mute", updatedAt: 30 },
    { muted: false, updatedAt: 20 },
  );
  assert.equal(state.level, "mute");
  assert.equal(state.hidden, true);
});

test("newer prefs 'mentions' beats a stale legacy mute", () => {
  const state = resolve(
    { level: "mentions", updatedAt: 30 },
    { muted: true, updatedAt: 20 },
  );
  assert.equal(state.level, "mentions");
});

test("prefs wins ties on the mute dimension", () => {
  const state = resolve(
    { level: "mentions", updatedAt: 20 },
    { muted: true, updatedAt: 20 },
  );
  assert.equal(state.level, "mentions");
});

test("a newer legacy unmute does not disturb non-mute prefs fields", () => {
  const state = resolve(
    { level: "mute", desktop: false, followAllThreads: true, updatedAt: 10 },
    { muted: false, updatedAt: 20 },
  );
  assert.equal(state.level, "all");
  assert.equal(state.desktop, false);
  assert.equal(state.followAllThreads, true);
});

test("a timed mute still applies over a newer legacy unmute", () => {
  const state = resolve(
    { level: "mute", muteUntil: NOW + 60, updatedAt: 10 },
    { muted: false, updatedAt: 20 },
  );
  assert.equal(state.level, "mute");
  assert.equal(state.timedMuteActive, true);
  assert.equal(state.hidden, false);
});

test("other channels' entries do not leak into the resolved state", () => {
  const state = resolveChannelNotifyState(
    "c",
    { version: 1, channels: { other: { level: "mute", updatedAt: 5 } } },
    { version: 1, channels: { other: { muted: true, updatedAt: 5 } } },
    NOW,
  );
  assert.equal(state, DEFAULT_CHANNEL_NOTIFY_STATE);
});

// ── nextTimedMuteExpiry ───────────────────────────────────────────────────────

test("nextTimedMuteExpiry: null when no timed mute is running", () => {
  assert.equal(nextTimedMuteExpiry({ version: 1, channels: {} }, NOW), null);
  assert.equal(
    nextTimedMuteExpiry(
      { version: 1, channels: { a: { muteUntil: NOW - 1, updatedAt: 1 } } },
      NOW,
    ),
    null,
  );
});

test("nextTimedMuteExpiry: earliest still-running expiry across channels", () => {
  const store = {
    version: 1,
    channels: {
      a: { muteUntil: NOW + 300, updatedAt: 1 },
      b: { muteUntil: NOW + 60, updatedAt: 1 },
      expired: { muteUntil: NOW - 10, updatedAt: 1 },
      none: { level: "mute", updatedAt: 1 },
    },
  };
  assert.equal(nextTimedMuteExpiry(store, NOW), NOW + 60);
});
