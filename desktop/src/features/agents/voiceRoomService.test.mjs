import assert from "node:assert/strict";
import test from "node:test";

import { getVoiceRoomSnapshot } from "./voiceSessionRegistry.ts";
import {
  executeVoiceRoomCommand,
  parseVoiceRoomCommandRequest,
  updateVoiceRoomCommandContext,
} from "./voiceRoomService.ts";

const architect = {
  agentName: "Architect",
  agentPubkey: "architect-pubkey",
  channelId: "architect-dm",
  mode: "proxy",
  relayUrl: "wss://relay.example",
  threadId: "architect-thread",
  voice: "cove",
};

test("controls an agent through the application room service", () => {
  updateVoiceRoomCommandContext({
    activeTargets: [],
    availableTargets: [architect],
  });

  assert.deepEqual(
    executeVoiceRoomCommand({ action: "join", agentName: "architect" }),
    {
      ok: true,
      action: "join",
      threadId: architect.threadId,
    },
  );
  assert.equal(getVoiceRoomSnapshot().activeTargets.length, 1);

  updateVoiceRoomCommandContext({
    activeTargets: getVoiceRoomSnapshot().activeTargets,
    availableTargets: [architect],
  });
  assert.equal(
    executeVoiceRoomCommand({
      action: "set-muted",
      agentPubkey: architect.agentPubkey,
      muted: true,
    }).ok,
    true,
  );
  assert.equal(getVoiceRoomSnapshot().activeTargets[0]?.muted, true);

  updateVoiceRoomCommandContext({
    activeTargets: getVoiceRoomSnapshot().activeTargets,
    availableTargets: [architect],
  });
  assert.equal(
    executeVoiceRoomCommand({
      action: "set-voice",
      threadId: architect.threadId,
      voice: "ember",
    }).ok,
    true,
  );
  assert.equal(getVoiceRoomSnapshot().activeTargets[0]?.voice, "ember");

  executeVoiceRoomCommand({ action: "set-output-muted", muted: true });
  assert.equal(getVoiceRoomSnapshot().outputMuted, true);

  updateVoiceRoomCommandContext({
    activeTargets: getVoiceRoomSnapshot().activeTargets,
    availableTargets: [architect],
  });
  assert.equal(
    executeVoiceRoomCommand({ action: "remove", agentName: "Architect" }).ok,
    true,
  );
  assert.equal(getVoiceRoomSnapshot().activeTargets.length, 0);
});

test("accepts only the narrow agent voice-room command envelope", () => {
  assert.deepEqual(
    parseVoiceRoomCommandRequest({
      type: "voice_room_command",
      requestId: "request-1",
      command: { action: "join", agentName: "Architect" },
    }),
    {
      type: "voice_room_command",
      requestId: "request-1",
      command: { action: "join", agentName: "Architect" },
    },
  );
  assert.equal(
    parseVoiceRoomCommandRequest({
      type: "voice_room_command",
      requestId: "request-2",
      command: { action: "remove", agentName: "Architect", shell: "rm" },
    }),
    null,
  );
});

test("rejects an unsupported voice instead of reporting a silent success", () => {
  updateVoiceRoomCommandContext({
    activeTargets: [architect],
    availableTargets: [architect],
  });
  assert.deepEqual(
    executeVoiceRoomCommand({
      action: "set-voice",
      agentName: "Architect",
      voice: "unknown",
    }),
    {
      ok: false,
      action: "set-voice",
      error: "Voice is not supported.",
    },
  );
});
