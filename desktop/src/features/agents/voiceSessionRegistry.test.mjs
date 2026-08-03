import assert from "node:assert/strict";
import test from "node:test";

import {
  addVoiceTarget,
  chooseAvailableVoice,
  endVoiceTarget,
  getVoiceRoomSnapshot,
  hasVoiceTarget,
  releaseVoiceRoomSpeaker,
  removeVoiceTarget,
  routeVoiceRoomTurn,
  startVoiceTarget,
} from "./voiceSessionRegistry.ts";

const solMedium = {
  agentName: "Sol [Medium]",
  agentPubkey: "medium",
  channelId: "dm-medium",
  mode: "native",
  relayUrl: "wss://relay.example",
  threadId: "thread-medium",
  voice: "sol",
};

const solHigh = {
  agentName: "Sol [High]",
  agentPubkey: "high",
  channelId: "dm-high",
  mode: "native",
  relayUrl: "wss://relay.example",
  threadId: "thread-high",
  voice: "cove",
};

test("keeps an active voice target independent of route ownership", () => {
  const active = addVoiceTarget([], solMedium);

  assert.equal(hasVoiceTarget(active, solMedium.threadId), true);
  assert.deepEqual(active, [solMedium]);
});

test("tracks concurrent voice targets for different agent tasks", () => {
  const active = addVoiceTarget(addVoiceTarget([], solMedium), solHigh);

  assert.deepEqual(active, [solMedium, solHigh]);
});

test("ending one voice target leaves the other session active", () => {
  const active = addVoiceTarget(addVoiceTarget([], solMedium), solHigh);

  assert.deepEqual(removeVoiceTarget(active, solMedium.threadId), [solHigh]);
});

test("assigns a distinct room voice before reusing the palette", () => {
  assert.equal(chooseAvailableVoice([]), "sol");
  assert.equal(chooseAvailableVoice([solMedium]), "cove");
  assert.equal(chooseAvailableVoice([solMedium, solHigh]), "ember");
});

test("assigns one recipient and one speaker lease per room turn", () => {
  startVoiceTarget(solMedium);
  startVoiceTarget(solHigh);
  const turn = routeVoiceRoomTurn("Sol High, please review this");

  assert.equal(turn?.recipientThreadId, solHigh.threadId);
  assert.deepEqual(getVoiceRoomSnapshot().speakerLease, {
    threadId: solHigh.threadId,
    turnId: turn?.id,
  });

  releaseVoiceRoomSpeaker(solHigh.threadId);
  assert.equal(getVoiceRoomSnapshot().speakerLease, null);
  endVoiceTarget(solMedium.threadId);
  endVoiceTarget(solHigh.threadId);
});
