import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_SENDER_SOUNDS,
  DEFAULT_SLOT_SOUNDS,
  SENDER_SOUND_SLOTS,
  SOUND_SLOTS,
  notificationSoundForSlot,
  resolveEventSound,
  resolveSlotSound,
} from "./sound.ts";

const prefs = {
  sounds: { ...DEFAULT_SLOT_SOUNDS, needs_action: "bong", dm: "doop" },
  senderSounds: { human: "flutter", agent: "amp" },
};

test("mentions use amp while other message slots resolve by sender kind", () => {
  for (const slot of SENDER_SOUND_SLOTS) {
    assert.equal(
      resolveEventSound(prefs, slot, false),
      slot === "mention" ? "amp" : "flutter",
    );
    assert.equal(resolveEventSound(prefs, slot, true), "amp");
  }
});

test("only mentions have an audible notification policy", () => {
  assert.equal(notificationSoundForSlot("mention"), "amp");
  assert.equal(notificationSoundForSlot("dm"), null);
  assert.equal(notificationSoundForSlot("thread_reply"), null);
  assert.equal(notificationSoundForSlot("needs_action"), null);
});

test("non-message slots keep their per-slot sound regardless of sender", () => {
  for (const slot of SOUND_SLOTS) {
    if (SENDER_SOUND_SLOTS.has(slot)) continue;
    assert.equal(resolveEventSound(prefs, slot, false), prefs.sounds[slot]);
    assert.equal(resolveEventSound(prefs, slot, true), prefs.sounds[slot]);
  }
});

test("resolveSlotSound ignores sender sounds", () => {
  assert.equal(resolveSlotSound(prefs, "dm"), "doop");
});

test("default sender sounds are human ping, agent amp", () => {
  assert.deepEqual(DEFAULT_SENDER_SOUNDS, { human: "ping", agent: "amp" });
});
