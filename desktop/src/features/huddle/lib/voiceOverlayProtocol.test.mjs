import assert from "node:assert/strict";
import test from "node:test";

import {
  parseVoiceOverlayAction,
  voiceOverlayMediaSnapshot,
} from "./voiceOverlayProtocol.ts";

test("parseVoiceOverlayAction accepts every supported controller action", () => {
  assert.deepEqual(
    parseVoiceOverlayAction({ version: 1, type: "toggle_mute" }),
    {
      version: 1,
      type: "toggle_mute",
    },
  );
  assert.deepEqual(
    parseVoiceOverlayAction({
      version: 1,
      type: "set_voice_input_mode",
      mode: "push_to_talk",
    }),
    { version: 1, type: "set_voice_input_mode", mode: "push_to_talk" },
  );
  assert.deepEqual(
    parseVoiceOverlayAction({
      version: 1,
      type: "set_voice_input_mode",
      mode: "voice_activity",
    }),
    { version: 1, type: "set_voice_input_mode", mode: "voice_activity" },
  );
  for (const type of [
    "toggle_transcription",
    "toggle_tts",
    "leave",
    "show_main",
  ]) {
    assert.deepEqual(parseVoiceOverlayAction({ version: 1, type }), {
      version: 1,
      type,
    });
  }
});

test("parseVoiceOverlayAction rejects malformed or unsupported window messages", () => {
  const invalidPayloads = [
    null,
    "toggle_mute",
    {},
    { type: "toggle_mute" },
    { version: 2, type: "toggle_mute" },
    { version: 1, type: "toggle_mute", unexpected: true },
    { version: 1, type: "set_voice_input_mode" },
    { version: 1, type: "set_voice_input_mode", mode: "always_on" },
    { version: 1, type: "leave", channelId: "another-channel" },
    { version: 1, type: "run_command", command: "rm -rf /" },
  ];

  for (const payload of invalidPayloads) {
    assert.equal(parseVoiceOverlayAction(payload), null);
  }
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
