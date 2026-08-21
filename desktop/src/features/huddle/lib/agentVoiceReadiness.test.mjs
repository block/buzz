import { strict as assert } from "node:assert";
import test from "node:test";

import { getAgentVoiceReadiness } from "./agentVoiceReadiness.ts";

const READY = {
  hasAgents: true,
  isMuted: false,
  isPttMode: true,
  micConnected: true,
  pushToTalkShortcut: "Ctrl+Space",
  transcriptionEnabled: true,
};

test("does not warn when no agent is enrolled", () => {
  assert.equal(
    getAgentVoiceReadiness({
      ...READY,
      hasAgents: false,
      isMuted: true,
    }),
    null,
  );
});

test("explains that agents need transcription", () => {
  assert.deepEqual(
    getAgentVoiceReadiness({ ...READY, transcriptionEnabled: false }),
    {
      action: "enable_transcription",
      message: "Transcript is off — turn it on so agents can hear you.",
    },
  );
});

test("makes muted push-to-talk visible with the platform shortcut", () => {
  assert.deepEqual(getAgentVoiceReadiness({ ...READY, isMuted: true }), {
    action: "unmute",
    message:
      "Mic muted — click to unmute or hold Ctrl+Space to talk to agents.",
  });
});

test("reports an unavailable microphone before accepting speech", () => {
  assert.deepEqual(getAgentVoiceReadiness({ ...READY, micConnected: false }), {
    action: null,
    message: "Microphone unavailable — agents cannot hear you.",
  });
});

test("does not warn after voice input is ready", () => {
  assert.equal(getAgentVoiceReadiness(READY), null);
});
