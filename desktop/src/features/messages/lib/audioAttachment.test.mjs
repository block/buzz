import assert from "node:assert/strict";
import test from "node:test";

import {
  formatVoiceNoteDuration,
  isAudioAttachment,
  isVoiceNoteAttachment,
  isVoiceNoteFile,
  nextVoiceNotePlaybackRate,
  resolveAudioAttachment,
  summarizeWaveform,
  VOICE_NOTE_MAX_DURATION_SECONDS,
  voiceNoteBarHeight,
  waveformPeaks,
  WAVEFORM_SUMMARY_RESOLUTION,
} from "./audioAttachment.ts";

test("isVoiceNoteFile scopes deferred audio uploads to recorder output", () => {
  assert.equal(
    isVoiceNoteFile(
      new File([new Uint8Array([1])], "voice-note-123.wav", {
        type: "audio/wav",
      }),
    ),
    true,
  );
  assert.equal(
    isVoiceNoteFile(
      new File([new Uint8Array([1])], "meeting.wav", { type: "audio/wav" }),
    ),
    false,
  );
});

test("resolveAudioAttachment accepts audio imeta and preserves metadata", () => {
  assert.deepEqual(
    resolveAudioAttachment(
      {
        duration: 12.5,
        filename: "voice-note.webm",
        m: "audio/webm;codecs=opus",
        size: 2048,
      },
      "https://relay.example/media/voice-note.webm",
      "Voice note",
    ),
    {
      duration: 12.5,
      filename: "voice-note.webm",
      href: "https://relay.example/media/voice-note.webm",
      size: 2048,
    },
  );
});

test("generic audio renders without triggering voice-note exclusivity", () => {
  const entry = {
    duration: 42,
    filename: "meeting.mp3",
    m: "audio/mpeg",
  };
  assert.equal(isVoiceNoteAttachment(entry), false);
  assert.equal(isAudioAttachment(entry), true);
  assert.deepEqual(
    resolveAudioAttachment(
      entry,
      "https://relay.example/media/meeting.mp3",
      "meeting.mp3",
    ),
    {
      duration: 42,
      filename: "meeting.mp3",
      href: "https://relay.example/media/meeting.mp3",
      size: undefined,
    },
  );
});

test("resolveAudioAttachment leaves non-audio files on the generic path", () => {
  assert.equal(
    resolveAudioAttachment(
      { filename: "notes.pdf", m: "application/pdf" },
      "https://relay.example/media/notes.pdf",
      "notes.pdf",
    ),
    null,
  );
});

test("packaged MP4 voice notes still resolve to the audio player", () => {
  const entry = {
    duration: 7.2,
    filename: "voice-note-123.mp4",
    m: "video/mp4",
  };
  assert.equal(isVoiceNoteAttachment(entry), true);
  assert.deepEqual(
    resolveAudioAttachment(
      entry,
      "https://relay.example/media/hash.mp4",
      "voice-note-123.mp4",
    ),
    {
      duration: 7.2,
      filename: "voice-note-123.mp4",
      href: "https://relay.example/media/hash.mp4",
      size: undefined,
    },
  );
  assert.equal(
    isVoiceNoteAttachment({ filename: "meeting.mp4", m: "video/mp4" }),
    false,
  );
});

test("formatVoiceNoteDuration formats minutes and seconds", () => {
  assert.equal(formatVoiceNoteDuration(0), "0:00");
  assert.equal(formatVoiceNoteDuration(65.9), "1:05");
});

test("voice notes have a five-minute recording limit", () => {
  assert.equal(VOICE_NOTE_MAX_DURATION_SECONDS, 300);
  assert.equal(
    formatVoiceNoteDuration(VOICE_NOTE_MAX_DURATION_SECONDS),
    "5:00",
  );
});

test("nextVoiceNotePlaybackRate follows the voice-note speed cycle", () => {
  assert.equal(nextVoiceNotePlaybackRate(1), 1.5);
  assert.equal(nextVoiceNotePlaybackRate(1.5), 2);
  assert.equal(nextVoiceNotePlaybackRate(2), 0.5);
  assert.equal(nextVoiceNotePlaybackRate(0.5), 1);
  assert.equal(nextVoiceNotePlaybackRate(99), 1);
});

test("waveformPeaks produces normalized accessible-height bars", () => {
  const peaks = waveformPeaks(new Float32Array([0, 0.25, -0.5, 1]), 2);
  assert.deepEqual(peaks, [0.25, 1]);
  assert.deepEqual(waveformPeaks(new Float32Array(), 2), [0.12, 0.12]);
});

test("voiceNoteBarHeight keeps quiet samples circular", () => {
  assert.equal(voiceNoteBarHeight(0), 3);
  assert.equal(voiceNoteBarHeight(0.12), 3);
  assert.equal(voiceNoteBarHeight(0.16), 3);
  assert.equal(voiceNoteBarHeight(1), 20);
});

test("summarizeWaveform bounds retained data regardless of clip length", () => {
  const longClip = new Float32Array(48_000 * 300).map(() => 0.5);
  const summary = summarizeWaveform(longClip);
  assert.equal(summary.length, WAVEFORM_SUMMARY_RESOLUTION);
  assert.ok(
    summary.length < longClip.length,
    "summary must be far smaller than the decoded clip",
  );

  const short = new Float32Array([0.2, 0.9]);
  assert.equal(summarizeWaveform(short).length, 2);
  assert.equal(summarizeWaveform(new Float32Array()).length, 1);
});

test("resampling the summary matches pooling the raw samples", () => {
  const samples = new Float32Array([0, 0.25, -0.5, 1, -0.3, 0.8]);
  const summary = summarizeWaveform(samples, 6);
  assert.deepEqual(
    Array.from(waveformPeaks(summary, 2)),
    Array.from(waveformPeaks(samples, 2)),
  );
});
