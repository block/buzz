import assert from "node:assert/strict";
import test from "node:test";

import {
  HUDDLE_REQUEST_RING_TIMEOUT_MS,
  createHuddleRequestRingtoneController,
  huddleIdFromLifecycleContent,
  huddleRingtoneCommand,
  shouldRingForHuddleRequest,
} from "./huddleRequestRingtone.ts";

function createHarness() {
  const audioElements = [];
  const timeouts = new Map();
  let nextTimeoutId = 1;

  const controller = createHuddleRequestRingtoneController({
    createAudio: (source) => {
      const audio = {
        currentTime: -1,
        loop: false,
        pauseCount: 0,
        playCount: 0,
        source,
        pause() {
          this.pauseCount += 1;
        },
        async play() {
          this.playCount += 1;
        },
      };
      audioElements.push(audio);
      return audio;
    },
    scheduleTimeout: (callback, delayMs) => {
      const id = nextTimeoutId++;
      timeouts.set(id, { callback, delayMs });
      return id;
    },
    clearScheduledTimeout: (id) => timeouts.delete(id),
  });

  return { audioElements, controller, timeouts };
}

test("eligible huddle requests start one looping ringtone", () => {
  const { audioElements, controller, timeouts } = createHarness();

  assert.equal(controller.start("huddle-a", "unison"), true);
  assert.equal(controller.start("huddle-a", "unison"), false);
  assert.equal(audioElements.length, 1);
  assert.equal(audioElements[0].source, "/sounds/unison.mp3");
  assert.equal(audioElements[0].loop, true);
  assert.equal(audioElements[0].currentTime, 0);
  assert.equal(audioElements[0].playCount, 1);
  assert.equal(timeouts.size, 1);
  assert.equal(
    [...timeouts.values()][0].delayMs,
    HUDDLE_REQUEST_RING_TIMEOUT_MS,
  );
});

test("a new huddle request replaces the ringtone already playing", () => {
  const { audioElements, controller, timeouts } = createHarness();

  controller.start("huddle-a", "unison");
  controller.start("huddle-b", "ping");

  assert.equal(audioElements.length, 2);
  assert.equal(audioElements[0].pauseCount, 1);
  assert.equal(audioElements[0].currentTime, 0);
  assert.equal(audioElements[1].source, "/sounds/ping.mp3");
  assert.equal(timeouts.size, 1);
});

test("only the matching huddle can stop an active ringtone", () => {
  const { audioElements, controller, timeouts } = createHarness();

  controller.start("huddle-a", "unison");
  assert.equal(controller.stop("huddle-b"), false);
  assert.equal(audioElements[0].pauseCount, 0);

  assert.equal(controller.stop("huddle-a"), true);
  assert.equal(audioElements[0].pauseCount, 1);
  assert.equal(audioElements[0].currentTime, 0);
  assert.equal(timeouts.size, 0);
});

test("the timeout stops a ringtone whose request received no response", () => {
  const { audioElements, controller, timeouts } = createHarness();

  controller.start("huddle-a", "unison");
  [...timeouts.values()][0].callback();

  assert.equal(audioElements[0].pauseCount, 1);
  assert.equal(timeouts.size, 0);
});

test("huddle ringtone policy excludes initiators, disabled alerts, and muted channels", () => {
  const base = {
    currentPubkey: "recipient",
    enabled: true,
    initiatorPubkey: "initiator",
    muted: false,
  };

  assert.equal(shouldRingForHuddleRequest(base), true);
  assert.equal(
    shouldRingForHuddleRequest({ ...base, currentPubkey: "INITIATOR" }),
    false,
  );
  assert.equal(shouldRingForHuddleRequest({ ...base, enabled: false }), false);
  assert.equal(shouldRingForHuddleRequest({ ...base, muted: true }), false);
  assert.equal(
    shouldRingForHuddleRequest({ ...base, currentPubkey: undefined }),
    false,
  );
});

test("huddle lifecycle content resolves the session that starts or stops ringing", () => {
  assert.equal(
    huddleIdFromLifecycleContent('{"ephemeral_channel_id":"huddle-a"}'),
    "huddle-a",
  );
  assert.equal(huddleIdFromLifecycleContent("{}"), null);
  assert.equal(huddleIdFromLifecycleContent("not-json"), null);
});

test("only huddle start and end events produce ringtone commands", () => {
  const content = '{"ephemeral_channel_id":"huddle-a"}';

  assert.deepEqual(huddleRingtoneCommand(48100, content), {
    action: "start",
    huddleId: "huddle-a",
  });
  assert.deepEqual(huddleRingtoneCommand(48103, content), {
    action: "stop",
    huddleId: "huddle-a",
  });
  assert.equal(huddleRingtoneCommand(48101, content), null);
  assert.equal(huddleRingtoneCommand(48100, "{}"), null);
});
