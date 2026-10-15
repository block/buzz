export type AudioAttachmentImetaEntry = {
  duration?: number;
  filename?: string;
  m?: string;
  size?: number;
};

export type ResolvedAudioAttachment = {
  duration?: number;
  filename: string;
  href: string;
  size?: number;
};

export const VOICE_NOTE_MAX_DURATION_SECONDS = 5 * 60;

export function isVoiceNoteFile(file: File): boolean {
  const filename = file.name.toLowerCase();
  return (
    file.type.startsWith("audio/") &&
    filename.startsWith("voice-note-") &&
    filename.endsWith(".wav")
  );
}

export function isVoiceNoteAttachment(
  entry: AudioAttachmentImetaEntry | undefined,
): boolean {
  const mime = entry?.m?.toLowerCase() ?? "";
  const filename = entry?.filename?.toLowerCase() ?? "";
  if (!filename.startsWith("voice-note-")) return false;
  if (mime.startsWith("audio/")) return true;
  return mime === "video/mp4" && filename.endsWith(".mp4");
}

export function isAudioAttachment(
  entry: AudioAttachmentImetaEntry | undefined,
): boolean {
  const mime = entry?.m?.toLowerCase() ?? "";
  return mime.startsWith("audio/") || isVoiceNoteAttachment(entry);
}

export function resolveAudioAttachment(
  entry: AudioAttachmentImetaEntry | undefined,
  href: string | undefined,
  childText: string,
): ResolvedAudioAttachment | null {
  if (!href || !entry || !isAudioAttachment(entry)) return null;

  return {
    duration: entry.duration,
    filename:
      entry.filename ||
      childText.trim() ||
      href.split("/").pop() ||
      "voice-note",
    href,
    size: entry.size,
  };
}

export function formatVoiceNoteDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
  const rounded = Math.floor(seconds);
  const minutes = Math.floor(rounded / 60);
  return `${minutes}:${String(rounded % 60).padStart(2, "0")}`;
}

export const VOICE_NOTE_PLAYBACK_RATES = [1, 1.5, 2, 0.5] as const;

export function nextVoiceNotePlaybackRate(currentRate: number): number {
  const currentIndex = VOICE_NOTE_PLAYBACK_RATES.indexOf(
    currentRate as (typeof VOICE_NOTE_PLAYBACK_RATES)[number],
  );
  return VOICE_NOTE_PLAYBACK_RATES[
    (currentIndex + 1) % VOICE_NOTE_PLAYBACK_RATES.length
  ];
}

const QUIET_LEVEL_THRESHOLD = 0.16;

// Waveform cards keep only this many peak buckets, not the full decoded clip.
// 256 matches the maximum bar count a card can display, so a resampled envelope
// is visually indistinguishable while retaining a fixed ~1KB regardless of clip
// duration (a 5-minute 48kHz mono note would otherwise pin ~57MB per card).
export const WAVEFORM_SUMMARY_RESOLUTION = 256;

// Reduce decoded PCM to a bounded peak envelope via max-pooling. Downstream
// display resamples this envelope to the (smaller) bar count; because 256 far
// exceeds the bars a card renders, the resampled result is visually
// indistinguishable from pooling the original samples directly.
export function summarizeWaveform(
  samples: Float32Array,
  resolution: number = WAVEFORM_SUMMARY_RESOLUTION,
): Float32Array {
  const buckets = Math.max(1, Math.min(resolution, samples.length || 1));
  const summary = new Float32Array(buckets);
  if (samples.length === 0) return summary;
  for (let index = 0; index < buckets; index += 1) {
    const start = Math.floor((index * samples.length) / buckets);
    const end = Math.max(
      start + 1,
      Math.floor(((index + 1) * samples.length) / buckets),
    );
    let peak = 0;
    for (let sampleIndex = start; sampleIndex < end; sampleIndex += 1) {
      peak = Math.max(peak, Math.abs(samples[sampleIndex] ?? 0));
    }
    summary[index] = peak;
  }
  return summary;
}

export function voiceNoteBarHeight(level: number): number {
  const audibleLevel = Math.max(
    0,
    (Math.min(1, level) - QUIET_LEVEL_THRESHOLD) / (1 - QUIET_LEVEL_THRESHOLD),
  );
  return 3 + Math.round(audibleLevel * 17);
}

export function waveformPeaks(
  samples: Float32Array,
  barCount: number,
): number[] {
  if (barCount <= 0) return [];
  if (samples.length === 0) return Array.from({ length: barCount }, () => 0.12);

  const peaks = Array.from({ length: barCount }, (_, index) => {
    const start = Math.floor((index * samples.length) / barCount);
    const end = Math.max(
      start + 1,
      Math.floor(((index + 1) * samples.length) / barCount),
    );
    let peak = 0;
    for (let sampleIndex = start; sampleIndex < end; sampleIndex += 1) {
      peak = Math.max(peak, Math.abs(samples[sampleIndex] ?? 0));
    }
    return peak;
  });
  const maximum = Math.max(...peaks, 0.001);
  return peaks.map((peak) => Math.max(0.12, Math.min(1, peak / maximum)));
}
