import assert from "node:assert/strict";
import test from "node:test";

const values = new Map();
const windowListeners = new Map();

globalThis.window = {
  addEventListener: (type, listener) => windowListeners.set(type, listener),
};
globalThis.localStorage = {
  getItem: (key) => values.get(key) ?? null,
  setItem: (key, value) => values.set(key, String(value)),
};

values.set("buzz.media.videoPlaybackSpeed", "2");

const preference = await import("./videoPlaybackSpeedPreference.ts");

test("reads the persisted speed as the starting preference", () => {
  assert.equal(preference.getVideoPlaybackSpeed(), 2);
});

test("defaults missing and unsupported speeds to 1x", () => {
  assert.equal(preference.parseVideoPlaybackSpeed(null), 1);
  assert.equal(preference.parseVideoPlaybackSpeed("3"), 1);
  assert.equal(preference.parseVideoPlaybackSpeed("fast"), 1);
  assert.equal(preference.parseVideoPlaybackSpeed("1.5"), 1.5);
});

test("persists a newly selected speed for later players", () => {
  preference.setVideoPlaybackSpeed(1.5);
  assert.equal(preference.getVideoPlaybackSpeed(), 1.5);
  assert.equal(values.get(preference.VIDEO_PLAYBACK_SPEED_STORAGE_KEY), "1.5");
});

test("ignores speeds the control cannot display", () => {
  preference.setVideoPlaybackSpeed(1.5);
  preference.setVideoPlaybackSpeed(3);
  assert.equal(preference.getVideoPlaybackSpeed(), 1.5);
  assert.equal(values.get(preference.VIDEO_PLAYBACK_SPEED_STORAGE_KEY), "1.5");
});

test("notifies subscribers when the speed changes", () => {
  let notifications = 0;
  const unsubscribe = preference.subscribeToVideoPlaybackSpeed(() => {
    notifications += 1;
  });

  preference.setVideoPlaybackSpeed(2);
  assert.equal(notifications, 1);

  // Re-selecting the same speed is not a change.
  preference.setVideoPlaybackSpeed(2);
  assert.equal(notifications, 1);

  unsubscribe();
  preference.setVideoPlaybackSpeed(1);
  assert.equal(notifications, 1);
});

test("notifies mounted consumers when another window changes the speed", () => {
  preference.setVideoPlaybackSpeed(1);
  let notifications = 0;
  const unsubscribe = preference.subscribeToVideoPlaybackSpeed(() => {
    notifications += 1;
  });

  values.set("buzz.media.videoPlaybackSpeed", "0.5");
  windowListeners.get("storage")({ key: "buzz.media.videoPlaybackSpeed" });
  assert.equal(preference.getVideoPlaybackSpeed(), 0.5);
  assert.equal(notifications, 1);

  // A redundant storage event must not needlessly re-render every player.
  windowListeners.get("storage")({ key: "buzz.media.videoPlaybackSpeed" });
  assert.equal(notifications, 1);
  unsubscribe();
});
