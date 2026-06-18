// Generates a waveform SVG next to each mp3 in the given directory, for the
// notification sound picker (SoundPicker.tsx renders them via CSS mask).
// Requires ffmpeg on PATH.
//
//   node scripts/generate-sound-waveforms.mjs public/sounds
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, readdirSync } from "node:fs";
import { join, basename } from "node:path";
import { tmpdir } from "node:os";

const SOUNDS_DIR = process.argv[2];
if (!SOUNDS_DIR) {
  console.error("usage: node generate-sound-waveforms.mjs <sounds-dir>");
  process.exit(1);
}

const BARS = 24;
const WIDTH = 200;
const HEIGHT = 80;
const BAR_WIDTH = 5;
const MAX_BAR = 76; // tallest bar, leaves vertical breathing room
const MIN_BAR = 4; // floor so quiet buckets render a short capsule
const GAMMA = 1.6; // contrast curve: deepens valleys, keeps peaks tall
const DETAIL_GAIN = 1.8; // unsharp mask on bar levels: amplifies each bar's deviation from its neighborhood so peaks push higher and valleys dip lower
const TAIL_FLOOR = 0.1; // windows quieter than this fraction of the loudest window are trimmed

const mp3s = readdirSync(SOUNDS_DIR)
  .filter((f) => f.endsWith(".mp3"))
  .sort();

for (const mp3 of mp3s) {
  const name = basename(mp3, ".mp3");
  const raw = join(tmpdir(), `${name}.s16le`);
  execFileSync("ffmpeg", [
    "-y",
    "-v",
    "error",
    "-i",
    join(SOUNDS_DIR, mp3),
    "-ac",
    "1",
    "-f",
    "s16le",
    "-acodec",
    "pcm_s16le",
    raw,
  ]);

  const buf = readFileSync(raw);
  const samples = new Int16Array(
    buf.buffer,
    buf.byteOffset,
    buf.byteLength / 2,
  );

  // File peak for normalization + silence threshold.
  let filePeak = 1;
  for (const s of samples) {
    const a = Math.abs(s);
    if (a > filePeak) filePeak = a;
  }

  // Trim by windowed RMS energy rather than single-sample threshold —
  // notification sounds carry long reverb tails that read as dead space.
  // Keep the region holding the audible body (windows above TAIL_FLOOR of
  // the loudest window), so the shape spans the full bar count.
  const WINDOW = 1024;
  const windowCount = Math.ceil(samples.length / WINDOW);
  const windowRms = new Array(windowCount).fill(0);
  for (let w = 0; w < windowCount; w++) {
    const from = w * WINDOW;
    const to = Math.min(samples.length, from + WINDOW);
    let sumSquares = 0;
    for (let j = from; j < to; j++) sumSquares += samples[j] * samples[j];
    windowRms[w] = Math.sqrt(sumSquares / Math.max(1, to - from));
  }
  const maxWindowRms = Math.max(...windowRms, 1);
  const tailThreshold = maxWindowRms * TAIL_FLOOR;
  let firstWindow = 0;
  while (
    firstWindow < windowCount - 1 &&
    windowRms[firstWindow] < tailThreshold
  )
    firstWindow++;
  let lastWindow = windowCount - 1;
  while (lastWindow > firstWindow && windowRms[lastWindow] < tailThreshold)
    lastWindow--;
  const trimmed = samples.subarray(
    firstWindow * WINDOW,
    Math.min(samples.length, (lastWindow + 1) * WINDOW),
  );

  // Sample a narrow window centered in each bar's bucket instead of
  // max-pooling the whole bucket — max-pooling erases amplitude modulation
  // (beats, double hits), which is what makes the shapes distinct. Then a
  // gamma curve to stretch the range.
  const peaks = new Array(BARS).fill(0);
  const bucketSize = Math.max(1, Math.floor(trimmed.length / BARS));
  const window = Math.max(64, Math.floor(bucketSize / 4));
  for (let i = 0; i < BARS; i++) {
    const center = i * bucketSize + Math.floor(bucketSize / 2);
    const from = Math.max(0, center - Math.floor(window / 2));
    const to = Math.min(trimmed.length, from + window);
    let peak = 0;
    for (let j = from; j < to; j++) {
      const a = Math.abs(trimmed[j]);
      if (a > peak) peak = a;
    }
    peaks[i] = peak;
  }
  const maxLevel = Math.max(...peaks, 1);
  for (let i = 0; i < BARS; i++) {
    peaks[i] = (peaks[i] / maxLevel) ** GAMMA;
  }

  // Local-contrast pass: push each bar away from the average of its
  // neighbors, then renormalize so the tallest bar still hits MAX_BAR.
  const sharpened = peaks.map((p, i) => {
    let sum = 0;
    let count = 0;
    for (let j = Math.max(0, i - 2); j <= Math.min(BARS - 1, i + 2); j++) {
      sum += peaks[j];
      count++;
    }
    const local = sum / count;
    return Math.max(0, p + DETAIL_GAIN * (p - local));
  });
  const maxSharpened = Math.max(...sharpened, 0.001);
  for (let i = 0; i < BARS; i++) {
    peaks[i] = Math.min(1, sharpened[i] / maxSharpened);
  }

  const step = WIDTH / BARS;
  const bars = peaks
    .map((p, i) => {
      const h = Math.max(MIN_BAR, p * MAX_BAR);
      const x = (i * step + (step - BAR_WIDTH) / 2).toFixed(2);
      const y = ((HEIGHT - h) / 2).toFixed(2);
      return `<rect x="${x}" y="${y}" width="${BAR_WIDTH}" height="${h.toFixed(2)}" rx="${(BAR_WIDTH / 2).toFixed(2)}"/>`;
    })
    .join("");

  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${WIDTH} ${HEIGHT}" fill="currentColor" aria-hidden="true">${bars}</svg>\n`;
  writeFileSync(join(SOUNDS_DIR, `${name}.svg`), svg);

  const seconds = (trimmed.length / 44100).toFixed(2);
  console.log(
    `${name}.svg  (${seconds}s audible, peak ${(filePeak / 32768).toFixed(2)})`,
  );
}
