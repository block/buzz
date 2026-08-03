import assert from "node:assert/strict";
import test from "node:test";

import * as voiceOverlayProtocol from "./voiceOverlayProtocol.ts";
import {
  parseVoiceOverlayAction,
  voiceOverlayMediaSnapshot,
} from "./voiceOverlayProtocol.ts";

test("parseVoiceOverlayAction accepts every supported controller action", () => {
  assert.deepEqual(
    parseVoiceOverlayAction({
      version: 1,
      requestId: "request-1",
      type: "toggle_mute",
    }),
    {
      version: 1,
      requestId: "request-1",
      type: "toggle_mute",
    },
  );
  assert.deepEqual(
    parseVoiceOverlayAction({
      version: 1,
      requestId: "request-2",
      type: "set_voice_input_mode",
      mode: "push_to_talk",
    }),
    {
      version: 1,
      requestId: "request-2",
      type: "set_voice_input_mode",
      mode: "push_to_talk",
    },
  );
  assert.deepEqual(
    parseVoiceOverlayAction({
      version: 1,
      requestId: "request-3",
      type: "set_voice_input_mode",
      mode: "voice_activity",
    }),
    {
      version: 1,
      requestId: "request-3",
      type: "set_voice_input_mode",
      mode: "voice_activity",
    },
  );
  for (const type of [
    "toggle_transcription",
    "toggle_tts",
    "leave",
    "show_main",
  ]) {
    assert.deepEqual(
      parseVoiceOverlayAction({
        version: 1,
        requestId: `request-${type}`,
        type,
      }),
      {
        version: 1,
        requestId: `request-${type}`,
        type,
      },
    );
  }
});

test("parseVoiceOverlayAction rejects malformed or unsupported window messages", () => {
  const invalidPayloads = [
    null,
    "toggle_mute",
    {},
    { type: "toggle_mute" },
    { version: 1, type: "toggle_mute" },
    { version: 2, requestId: "request-1", type: "toggle_mute" },
    { version: 1, requestId: "", type: "toggle_mute" },
    {
      version: 1,
      requestId: "request-1",
      type: "toggle_mute",
      unexpected: true,
    },
    { version: 1, requestId: "request-1", type: "set_voice_input_mode" },
    {
      version: 1,
      requestId: "request-1",
      type: "set_voice_input_mode",
      mode: "always_on",
    },
    {
      version: 1,
      requestId: "request-1",
      type: "leave",
      channelId: "another-channel",
    },
    {
      version: 1,
      requestId: "request-1",
      type: "run_command",
      command: "rm -rf /",
    },
  ];

  for (const payload of invalidPayloads) {
    assert.equal(parseVoiceOverlayAction(payload), null);
  }
});

test("parseVoiceOverlayActionResult accepts strict success and failure acknowledgements", () => {
  assert.equal(
    typeof voiceOverlayProtocol.parseVoiceOverlayActionResult,
    "function",
  );
  const parseVoiceOverlayActionResult =
    voiceOverlayProtocol.parseVoiceOverlayActionResult;

  assert.deepEqual(
    parseVoiceOverlayActionResult({
      version: 1,
      requestId: "request-1",
      ok: true,
    }),
    { version: 1, requestId: "request-1", ok: true },
  );
  assert.deepEqual(
    parseVoiceOverlayActionResult({
      version: 1,
      requestId: "request-2",
      ok: false,
      error: "Transcript failed",
    }),
    {
      version: 1,
      requestId: "request-2",
      ok: false,
      error: "Transcript failed",
    },
  );

  for (const payload of [
    null,
    { version: 1, requestId: "", ok: true },
    { version: 1, requestId: "request-1", ok: false },
    { version: 1, requestId: "request-1", ok: true, error: "unexpected" },
    { version: 1, requestId: "request-1", ok: false, error: "" },
  ]) {
    assert.equal(parseVoiceOverlayActionResult(payload), null);
  }
});

test("voice action tracking accepts a late acknowledgement after the slow warning", () => {
  assert.equal(
    typeof voiceOverlayProtocol.createVoiceOverlayActionTracker,
    "function",
  );
  const scheduled = new Map();
  const cleared = [];
  const slow = [];
  const expired = [];
  let nextTimerId = 1;
  const tracker = voiceOverlayProtocol.createVoiceOverlayActionTracker({
    setTimer(callback, delayMs) {
      const timerId = nextTimerId++;
      scheduled.set(timerId, { callback, delayMs });
      return timerId;
    },
    clearTimer(timerId) {
      cleared.push(timerId);
      scheduled.delete(timerId);
    },
    onSlow(requestId) {
      slow.push(requestId);
    },
    onExpired(requestId) {
      expired.push(requestId);
    },
  });

  tracker.start("request-1");
  assert.deepEqual(
    [...scheduled.values()].map(({ delayMs }) => delayMs),
    [2_000, 30_000],
  );
  scheduled.get(1).callback();

  assert.deepEqual(slow, ["request-1"]);
  assert.deepEqual(expired, []);
  assert.equal(tracker.settle("request-1"), true);
  assert.deepEqual(cleared, [1, 2]);
  assert.equal(tracker.settle("request-1"), false);
});

test("voice action tracking expires unanswered requests and disposes timers", () => {
  assert.equal(
    typeof voiceOverlayProtocol.createVoiceOverlayActionTracker,
    "function",
  );
  const scheduled = new Map();
  const cleared = [];
  const expired = [];
  let nextTimerId = 1;
  const tracker = voiceOverlayProtocol.createVoiceOverlayActionTracker({
    setTimer(callback, delayMs) {
      const timerId = nextTimerId++;
      scheduled.set(timerId, { callback, delayMs });
      return timerId;
    },
    clearTimer(timerId) {
      cleared.push(timerId);
      scheduled.delete(timerId);
    },
    onSlow() {},
    onExpired(requestId) {
      expired.push(requestId);
    },
  });

  tracker.start("request-1");
  scheduled.get(2).callback();

  assert.deepEqual(expired, ["request-1"]);
  assert.equal(tracker.settle("request-1"), false);

  tracker.start("request-2");
  tracker.dispose();
  assert.deepEqual(cleared, [1, 2, 3, 4]);
});

test("voice action tracking suppresses callbacks after an early acknowledgement", () => {
  const scheduled = new Map();
  const slow = [];
  const expired = [];
  let nextTimerId = 1;
  const tracker = voiceOverlayProtocol.createVoiceOverlayActionTracker({
    setTimer(callback, delayMs) {
      const timerId = nextTimerId++;
      scheduled.set(timerId, { callback, delayMs });
      return timerId;
    },
    clearTimer() {},
    onSlow(requestId) {
      slow.push(requestId);
    },
    onExpired(requestId) {
      expired.push(requestId);
    },
  });

  tracker.start("request-1");
  assert.equal(tracker.settle("request-1"), true);
  scheduled.get(1).callback();
  scheduled.get(2).callback();

  assert.deepEqual(slow, []);
  assert.deepEqual(expired, []);
});

test("voice action execution reports falsy handler rejections as failures", async () => {
  assert.equal(typeof voiceOverlayProtocol.runVoiceOverlayAction, "function");
  const action = {
    version: 1,
    requestId: "request-falsy-error",
    type: "toggle_mute",
  };

  for (const rejection of [undefined, null, "", 0, false]) {
    const result = await voiceOverlayProtocol.runVoiceOverlayAction(action, {
      onToggleMute: () => Promise.reject(rejection),
      onSetVoiceInputMode: () => {},
      onToggleTranscription: () => {},
      onToggleTts: () => {},
      onLeave: () => {},
      onShowMain: () => {},
    });
    assert.equal(result.version, 1);
    assert.equal(result.requestId, "request-falsy-error");
    assert.equal(result.ok, false);
    assert.equal(typeof result.error, "string");
    assert.ok(result.error.length > 0);
  }
});

test("voice action execution returns one matching success acknowledgement", async () => {
  let calls = 0;
  const result = await voiceOverlayProtocol.runVoiceOverlayAction(
    {
      version: 1,
      requestId: "request-success",
      type: "toggle_transcription",
    },
    {
      onToggleMute: () => {},
      onSetVoiceInputMode: () => {},
      onToggleTranscription: () => {
        calls += 1;
      },
      onToggleTts: () => {},
      onLeave: () => {},
      onShowMain: () => {},
    },
  );

  assert.equal(calls, 1);
  assert.deepEqual(result, {
    version: 1,
    requestId: "request-success",
    ok: true,
  });
});

test("voiceOverlayMediaSnapshot clamps microphone levels and hides activity while muted", () => {
  assert.deepEqual(
    voiceOverlayMediaSnapshot({
      version: 1,
      phase: "active",
      participantCount: 3,
      agentCount: 1,
      ttsEnabled: true,
      transcriptionEnabled: true,
      isLeaving: false,
      error: null,
      isMuted: false,
      micConnected: true,
      micLevel: 1.7,
      pttActive: true,
      voiceInputMode: "push_to_talk",
    }),
    {
      version: 1,
      phase: "active",
      participantCount: 3,
      agentCount: 1,
      ttsEnabled: true,
      transcriptionEnabled: true,
      isLeaving: false,
      error: null,
      isMuted: false,
      micConnected: true,
      micLevel: 1,
      pttActive: true,
      voiceInputMode: "push_to_talk",
    },
  );

  assert.deepEqual(
    voiceOverlayMediaSnapshot({
      version: 1,
      phase: "connected",
      participantCount: 2,
      agentCount: 0,
      ttsEnabled: false,
      transcriptionEnabled: false,
      isLeaving: true,
      error: "Voice connection interrupted",
      isMuted: true,
      micConnected: true,
      micLevel: 0.8,
      pttActive: true,
      voiceInputMode: "voice_activity",
    }),
    {
      version: 1,
      phase: "connected",
      participantCount: 2,
      agentCount: 0,
      ttsEnabled: false,
      transcriptionEnabled: false,
      isLeaving: true,
      error: "Voice connection interrupted",
      isMuted: true,
      micConnected: true,
      micLevel: 0,
      pttActive: false,
      voiceInputMode: "voice_activity",
    },
  );
});

test("voiceOverlayMediaSnapshot clears stale media state when the huddle becomes idle", () => {
  assert.deepEqual(
    voiceOverlayMediaSnapshot({
      version: 1,
      phase: "idle",
      participantCount: 3,
      agentCount: 1,
      ttsEnabled: true,
      transcriptionEnabled: true,
      isLeaving: true,
      error: null,
      isMuted: false,
      micConnected: true,
      micLevel: 0.8,
      pttActive: true,
      voiceInputMode: "push_to_talk",
    }),
    {
      version: 1,
      phase: "idle",
      participantCount: 0,
      agentCount: 0,
      ttsEnabled: true,
      transcriptionEnabled: true,
      isLeaving: false,
      error: null,
      isMuted: false,
      micConnected: false,
      micLevel: 0,
      pttActive: false,
      voiceInputMode: "push_to_talk",
    },
  );
});
