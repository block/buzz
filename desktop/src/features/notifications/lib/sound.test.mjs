import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import test from "node:test";

import { SOUND_NAMES, shouldPlayNotificationSound } from "./sound.ts";

test("every notification sound has playable audio and a waveform", () => {
  for (const name of SOUND_NAMES) {
    assert.equal(
      existsSync(
        new URL(`../../../../public/sounds/${name}.mp3`, import.meta.url),
      ),
      true,
      `${name} is missing audio`,
    );
    assert.equal(
      existsSync(
        new URL(`../../../../public/sounds/${name}.svg`, import.meta.url),
      ),
      true,
      `${name} is missing a waveform`,
    );
  }
});

test("silences notifications from Huddle backing channels", () => {
  const silentChannelIds = new Set(["active-huddle"]);

  assert.equal(
    shouldPlayNotificationSound("active-huddle", silentChannelIds),
    false,
  );
  assert.equal(
    shouldPlayNotificationSound("ordinary-channel", silentChannelIds),
    true,
  );
  assert.equal(shouldPlayNotificationSound(null, silentChannelIds), true);
});
