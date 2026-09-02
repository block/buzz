import assert from "node:assert/strict";
import test from "node:test";

import { encodeVoiceNoteWav } from "./voiceNoteWav.ts";

test("encodeVoiceNoteWav emits canonical mono PCM with no metadata chunks", () => {
  const bytes = encodeVoiceNoteWav(
    [new Float32Array([0, 0.5, -0.5, 1])],
    48_000,
    24_000,
  );
  const view = new DataView(bytes.buffer);
  const ascii = (start, length) =>
    String.fromCharCode(...bytes.slice(start, start + length));

  assert.equal(ascii(0, 4), "RIFF");
  assert.equal(ascii(8, 4), "WAVE");
  assert.equal(ascii(12, 4), "fmt ");
  assert.equal(ascii(36, 4), "data");
  assert.equal(view.getUint32(4, true), bytes.length - 8);
  assert.equal(view.getUint16(20, true), 1);
  assert.equal(view.getUint16(22, true), 1);
  assert.equal(view.getUint32(24, true), 24_000);
  assert.equal(view.getUint16(34, true), 16);
  assert.equal(bytes.length, 48);
});

test("encodeVoiceNoteWav mixes stereo into mono", () => {
  const bytes = encodeVoiceNoteWav(
    [new Float32Array([1]), new Float32Array([-1])],
    24_000,
    24_000,
  );
  assert.equal(new DataView(bytes.buffer).getInt16(44, true), 0);
});

test("encodeVoiceNoteWav rejects empty recordings", () => {
  assert.throws(() => encodeVoiceNoteWav([], 48_000), /empty voice note/);
});
