/**
 * Animated avatar capture pipeline.
 *
 * Records a short clip from the user's camera, removes the background per
 * frame with MediaPipe selfie segmentation (so only the person remains, with
 * soft alpha edges), and composes each frame as a "sticker": the person pops
 * out of a colored backdrop disc that sits inside the avatar circle. The
 * result is encoded as a ping-pong looping animated PNG (APNG) — full 24-bit
 * color and 8-bit alpha, unlike GIF — plus a static poster frame.
 *
 * Segmentation assets (wasm + model) are fetched lazily from public CDNs the
 * first time the feature is used. If they can't be loaded (e.g. offline),
 * recording still works — the background just isn't removed.
 */

import type { ImageSegmenter } from "@mediapipe/tasks-vision";
import UPNG from "upng-js";

export const ANIMATED_AVATAR_SIZE = 256;
const ANIMATED_AVATAR_CAPTURE_SIZE = 512;
export const ANIMATED_AVATAR_FPS = 12;
export const ANIMATED_AVATAR_DURATION_MS = 3000;
export const ANIMATED_AVATAR_FRAME_COUNT = Math.round(
  (ANIMATED_AVATAR_DURATION_MS / 1000) * ANIMATED_AVATAR_FPS,
);
export const ANIMATED_AVATAR_FRAME_DELAY_MS = Math.round(
  1000 / ANIMATED_AVATAR_FPS,
);

// Preserve truecolor pixels in the final APNG. Palette quantization made
// high-detail dark areas, like hair, shimmer between frames.
const APNG_COLOR_COUNT = 0;

// Soft alpha ramp for the segmentation confidence: fully transparent below
// the low bound, fully opaque above the high bound, feathered in between.
const PERSON_CONFIDENCE_LOW = 0.32;
const PERSON_CONFIDENCE_HIGH = 0.7;

// Dark camera pixels tend to carry sensor noise, which reads as shimmer once
// looped. Blend only low-luma pixels that barely changed from the previous
// frame, so hair noise settles without smearing real motion.
const DARK_DETAIL_DENOISE_LUMA_LOW = 24;
const DARK_DETAIL_DENOISE_LUMA_HIGH = 150;
const DARK_DETAIL_DENOISE_DIFF_LOW = 4;
const DARK_DETAIL_DENOISE_DIFF_HIGH = 32;
const DARK_DETAIL_DENOISE_MAX_BLEND = 0.52;

// Stabilize tiny frame-to-frame mask confidence changes around fine edges
// (hair especially), while letting larger movement update immediately.
const MASK_TEMPORAL_MAX_BLEND = 0.64;
const MASK_TEMPORAL_MOTION_THRESHOLD = 0.26;

/**
 * How the recorded person and the backdrop circle are placed in the frame.
 * Offsets are in 256x256 frame coordinates. The person's scale multiplies
 * the frame size (the default draws them slightly oversized and
 * bottom-anchored so their head pops above the backdrop); the circle's
 * scale multiplies its base geometry around its own center.
 */
export type AvatarComposition = {
  backdropColor: string | null;
  offsetX: number;
  offsetY: number;
  personOutline: boolean;
  scale: number;
  shapeOffsetX: number;
  shapeOffsetY: number;
  shapeScale: number;
};

// Default framing tuned by hand against real recordings.
export const DEFAULT_PERSON_SCALE = 1.26;
export const DEFAULT_PERSON_OFFSET_X = 1;
export const DEFAULT_PERSON_OFFSET_Y = 7;
export const DEFAULT_PERSON_OUTLINE = true;
export const MIN_PERSON_SCALE = 0.7;
export const MAX_PERSON_SCALE = 2;

export const DEFAULT_SHAPE_SCALE = 1.12;
export const DEFAULT_SHAPE_OFFSET_X = 0;
export const DEFAULT_SHAPE_OFFSET_Y = -7;
export const MIN_SHAPE_SCALE = 0;
export const MAX_SHAPE_SCALE = 1.5;

// Backdrop circle geometry (in 256x256 frame coordinates). The circle sits
// low in the frame and stays inside the inscribed circle that avatar
// components crop to; the pop-out clip extends a column from the circle's
// midline to the frame top so the person can rise out of the circle's top
// without spilling past its sides.
const CIRCLE_GEOMETRY = {
  centerX: 128,
  centerY: 152,
  radius: 100,
};
const PERSON_OUTLINE_ALPHA = 0.92;
const PERSON_OUTLINE_RADIUS = 2.75;
const PERSON_OUTLINE_OFFSETS = [
  [0, -PERSON_OUTLINE_RADIUS],
  [PERSON_OUTLINE_RADIUS, 0],
  [0, PERSON_OUTLINE_RADIUS],
  [-PERSON_OUTLINE_RADIUS, 0],
  [PERSON_OUTLINE_RADIUS * 0.72, -PERSON_OUTLINE_RADIUS * 0.72],
  [PERSON_OUTLINE_RADIUS * 0.72, PERSON_OUTLINE_RADIUS * 0.72],
  [-PERSON_OUTLINE_RADIUS * 0.72, PERSON_OUTLINE_RADIUS * 0.72],
  [-PERSON_OUTLINE_RADIUS * 0.72, -PERSON_OUTLINE_RADIUS * 0.72],
] as const;

// Pinned to the installed @mediapipe/tasks-vision version so the wasm loader
// always matches the JS API.
const MEDIAPIPE_WASM_BASE =
  "https://cdn.jsdelivr.net/npm/@mediapipe/tasks-vision@0.10.35/wasm";
const SELFIE_SEGMENTER_MODEL_URL =
  "https://storage.googleapis.com/mediapipe-models/image_segmenter/selfie_segmenter/float16/latest/selfie_segmenter.tflite";

export type AnimatedAvatarRecording = {
  /** Square RGBA cut-out frames, mirrored like a selfie preview. */
  frames: ImageData[];
  /** False when the segmentation model couldn't be loaded. */
  backgroundRemoved: boolean;
};

export type AvatarCameraDevice = {
  deviceId: string;
  label: string;
};

export async function listAvatarCameras(): Promise<AvatarCameraDevice[]> {
  if (!navigator.mediaDevices?.enumerateDevices) {
    return [];
  }

  const devices = await navigator.mediaDevices.enumerateDevices();
  return devices
    .filter((device) => device.kind === "videoinput" && device.deviceId)
    .map((device) => ({
      deviceId: device.deviceId,
      label: device.label,
    }));
}

export async function openAvatarCamera(
  deviceId?: string | null,
): Promise<MediaStream> {
  const video: MediaTrackConstraints = {
    facingMode: { ideal: "user" },
    frameRate: { ideal: 30 },
    height: { ideal: 1080 },
    width: { ideal: 1080 },
  };
  if (deviceId) {
    video.deviceId = { exact: deviceId };
  }

  return navigator.mediaDevices.getUserMedia({
    audio: false,
    video,
  });
}

export function stopAvatarCamera(stream: MediaStream): void {
  for (const track of stream.getTracks()) {
    track.stop();
  }
}

type SegmenterHandle = {
  segmenter: ImageSegmenter;
  /** Which confidence-mask channel holds the person. */
  personChannel: number;
  /** True when the channel is a background mask and must be inverted. */
  invert: boolean;
};

let segmenterPromise: Promise<SegmenterHandle | null> | null = null;

/**
 * Lazily create the selfie segmenter. Resolves to null when the CDN assets
 * are unreachable; a failed load is retried on the next call.
 */
function loadSegmenter(): Promise<SegmenterHandle | null> {
  if (!segmenterPromise) {
    segmenterPromise = createSegmenter().catch(() => {
      segmenterPromise = null;
      return null;
    });
  }
  return segmenterPromise;
}

async function createSegmenter(): Promise<SegmenterHandle> {
  const vision = await import("@mediapipe/tasks-vision");
  const fileset =
    await vision.FilesetResolver.forVisionTasks(MEDIAPIPE_WASM_BASE);
  const segmenter = await vision.ImageSegmenter.createFromOptions(fileset, {
    baseOptions: { modelAssetPath: SELFIE_SEGMENTER_MODEL_URL },
    outputCategoryMask: false,
    outputConfidenceMasks: true,
    runningMode: "VIDEO",
  });

  // The selfie model labels vary across releases ("background"/"person" vs a
  // single foreground channel) — resolve which channel is the person once.
  const labels = segmenter.getLabels().map((label) => label.toLowerCase());
  let personChannel = labels.findIndex(
    (label) =>
      label.includes("person") ||
      label.includes("selfie") ||
      label.includes("foreground"),
  );
  let invert = false;
  if (personChannel < 0) {
    personChannel = 0;
    invert =
      labels.length === 1 && (labels[0]?.includes("background") ?? false);
  }

  return { invert, personChannel, segmenter };
}

/** Warm the segmentation model while the user lines up their shot. */
export function preloadAvatarSegmenter(): void {
  void loadSegmenter();
}

export async function recordAnimatedAvatarFrames(
  video: HTMLVideoElement,
  options: {
    signal?: AbortSignal;
    onProgress?: (fraction: number) => void;
  } = {},
): Promise<AnimatedAvatarRecording> {
  const { onProgress, signal } = options;
  const handle = await loadSegmenter();

  const canvas = document.createElement("canvas");
  canvas.width = ANIMATED_AVATAR_CAPTURE_SIZE;
  canvas.height = ANIMATED_AVATAR_CAPTURE_SIZE;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) {
    throw new Error("Could not create a drawing context for recording.");
  }

  const frames: ImageData[] = [];
  let lastTimestamp = 0;
  let previousDenoisedFrame: Uint8ClampedArray | null = null;
  let previousMask: Float32Array | null = null;

  for (let i = 0; i < ANIMATED_AVATAR_FRAME_COUNT; i++) {
    if (signal?.aborted) {
      throw new DOMException("Recording cancelled", "AbortError");
    }

    const frameStart = performance.now();
    drawMirroredCenterCrop(context, video);
    const frame = context.getImageData(
      0,
      0,
      ANIMATED_AVATAR_CAPTURE_SIZE,
      ANIMATED_AVATAR_CAPTURE_SIZE,
    );
    previousDenoisedFrame = stabilizeDarkDetailNoise(
      frame,
      previousDenoisedFrame,
    );

    if (handle) {
      // Video-mode segmentation requires strictly increasing timestamps.
      let timestamp = performance.now();
      if (timestamp <= lastTimestamp) {
        timestamp = lastTimestamp + 1;
      }
      lastTimestamp = timestamp;
      const result = handle.segmenter.segmentForVideo(canvas, timestamp);
      try {
        previousMask = applyPersonMask(
          frame,
          result.confidenceMasks,
          handle,
          previousMask,
        );
      } finally {
        result.close();
      }
    }

    frames.push(frame);
    onProgress?.((i + 1) / ANIMATED_AVATAR_FRAME_COUNT);

    const elapsed = performance.now() - frameStart;
    await sleep(Math.max(0, ANIMATED_AVATAR_FRAME_DELAY_MS - elapsed));
  }

  return { backgroundRemoved: handle !== null, frames };
}

type ConfidenceMasks =
  | readonly {
      getAsFloat32Array(): Float32Array;
      width: number;
      height: number;
    }[]
  | undefined;

function applyPersonMask(
  frame: ImageData,
  masks: ConfidenceMasks,
  handle: SegmenterHandle,
  previousMask: Float32Array | null,
): Float32Array | null {
  const mask = masks?.[handle.personChannel] ?? masks?.[0];
  if (!mask) {
    return previousMask;
  }

  const values = mask.getAsFloat32Array();
  const pixels = frame.data;
  const scaleX = mask.width / frame.width;
  const scaleY = mask.height / frame.height;
  const rampRange = PERSON_CONFIDENCE_HIGH - PERSON_CONFIDENCE_LOW;
  const nextMask = new Float32Array(frame.width * frame.height);
  const canSmoothMask = previousMask?.length === nextMask.length;

  for (let y = 0; y < frame.height; y++) {
    for (let x = 0; x < frame.width; x++) {
      const confidence = sampleMaskConfidence(
        values,
        mask.width,
        mask.height,
        x * scaleX,
        y * scaleY,
      );
      let person = handle.invert ? 1 - confidence : confidence;
      const pixelIndex = y * frame.width + x;
      if (canSmoothMask) {
        person = stabilizeMaskConfidence(
          person,
          previousMask[pixelIndex] ?? person,
        );
      }
      nextMask[pixelIndex] = person;
      const offset = (y * frame.width + x) * 4;
      if (person <= PERSON_CONFIDENCE_LOW) {
        pixels[offset] = 0;
        pixels[offset + 1] = 0;
        pixels[offset + 2] = 0;
        pixels[offset + 3] = 0;
      } else if (person < PERSON_CONFIDENCE_HIGH) {
        // Feathered edge — APNG keeps the 8-bit alpha, so the cut-out blends
        // smoothly instead of GIF-style jagged edges.
        const alpha = (person - PERSON_CONFIDENCE_LOW) / rampRange;
        pixels[offset + 3] = Math.round((pixels[offset + 3] ?? 255) * alpha);
      }
    }
  }

  return nextMask;
}

function stabilizeDarkDetailNoise(
  frame: ImageData,
  previousFrame: Uint8ClampedArray | null,
): Uint8ClampedArray {
  const pixels = frame.data;
  const nextFrame = new Uint8ClampedArray(pixels);
  if (!previousFrame || previousFrame.length !== pixels.length) {
    return nextFrame;
  }

  for (let offset = 0; offset < pixels.length; offset += 4) {
    const red = pixels[offset] ?? 0;
    const green = pixels[offset + 1] ?? 0;
    const blue = pixels[offset + 2] ?? 0;
    const luma = 0.2126 * red + 0.7152 * green + 0.0722 * blue;
    const darkWeight = clamp01(
      (DARK_DETAIL_DENOISE_LUMA_HIGH - luma) /
        (DARK_DETAIL_DENOISE_LUMA_HIGH - DARK_DETAIL_DENOISE_LUMA_LOW),
    );
    if (darkWeight <= 0) {
      continue;
    }

    const previousRed = previousFrame[offset] ?? red;
    const previousGreen = previousFrame[offset + 1] ?? green;
    const previousBlue = previousFrame[offset + 2] ?? blue;
    const colorDiff =
      (Math.abs(red - previousRed) +
        Math.abs(green - previousGreen) +
        Math.abs(blue - previousBlue)) /
      3;
    const stabilityWeight = clamp01(
      (DARK_DETAIL_DENOISE_DIFF_HIGH - colorDiff) /
        (DARK_DETAIL_DENOISE_DIFF_HIGH - DARK_DETAIL_DENOISE_DIFF_LOW),
    );
    if (stabilityWeight <= 0) {
      continue;
    }

    const blend = DARK_DETAIL_DENOISE_MAX_BLEND * darkWeight * stabilityWeight;
    pixels[offset] = Math.round(red * (1 - blend) + previousRed * blend);
    pixels[offset + 1] = Math.round(
      green * (1 - blend) + previousGreen * blend,
    );
    pixels[offset + 2] = Math.round(blue * (1 - blend) + previousBlue * blend);
    nextFrame[offset] = pixels[offset] ?? red;
    nextFrame[offset + 1] = pixels[offset + 1] ?? green;
    nextFrame[offset + 2] = pixels[offset + 2] ?? blue;
  }

  return nextFrame;
}

function stabilizeMaskConfidence(current: number, previous: number): number {
  const delta = Math.abs(current - previous);
  const blend =
    MASK_TEMPORAL_MAX_BLEND *
    Math.max(0, 1 - delta / MASK_TEMPORAL_MOTION_THRESHOLD);
  return previous * blend + current * (1 - blend);
}

function clamp01(value: number): number {
  return Math.min(1, Math.max(0, value));
}

function drawMirroredCenterCrop(
  context: CanvasRenderingContext2D,
  video: HTMLVideoElement,
): void {
  const sourceSize = Math.min(video.videoWidth, video.videoHeight) || 1;
  const sourceX = (video.videoWidth - sourceSize) / 2;
  const sourceY = (video.videoHeight - sourceSize) / 2;

  context.save();
  context.clearRect(
    0,
    0,
    ANIMATED_AVATAR_CAPTURE_SIZE,
    ANIMATED_AVATAR_CAPTURE_SIZE,
  );
  // Mirror horizontally so the capture matches the selfie-style live preview.
  context.translate(ANIMATED_AVATAR_CAPTURE_SIZE, 0);
  context.scale(-1, 1);
  context.drawImage(
    video,
    sourceX,
    sourceY,
    sourceSize,
    sourceSize,
    0,
    0,
    ANIMATED_AVATAR_CAPTURE_SIZE,
    ANIMATED_AVATAR_CAPTURE_SIZE,
  );
  context.restore();
}

/** Convert recorded frames into bitmaps for fast repeated composition. */
export function createAvatarFrameBitmaps(
  frames: ImageData[],
): Promise<ImageBitmap[]> {
  return Promise.all(frames.map((frame) => createImageBitmap(frame)));
}

/**
 * Draw one composed avatar frame: the cut-out person, placed per the
 * composition's offset/scale, popping out of the backdrop circle. With no
 * backdrop color the placed cut-out renders on its own. The context must
 * belong to a 256x256 canvas.
 */
export function composeAvatarFrame(
  context: CanvasRenderingContext2D,
  person: CanvasImageSource,
  composition: AvatarComposition,
): void {
  const size = ANIMATED_AVATAR_SIZE;
  context.clearRect(0, 0, size, size);

  // Person placement: centered horizontally and bottom-anchored at the
  // default, then shifted/scaled by the user's framing choices.
  const personSize = size * composition.scale;
  const personX = (size - personSize) / 2 + composition.offsetX;
  const personY = size - personSize + composition.offsetY;

  if (!composition.backdropColor || composition.shapeScale <= 0) {
    drawPersonWithOutline(
      context,
      person,
      composition,
      personX,
      personY,
      personSize,
    );
    return;
  }

  // The circle scales around its own center, then shifts by the user's
  // offsets.
  const geometry = CIRCLE_GEOMETRY;
  const shapeScale = composition.shapeScale;
  const circleX = geometry.centerX + composition.shapeOffsetX;
  const circleY = geometry.centerY + composition.shapeOffsetY;
  const circleRadius = geometry.radius * shapeScale;

  context.fillStyle = composition.backdropColor;
  context.beginPath();
  context.arc(circleX, circleY, circleRadius, 0, Math.PI * 2);
  context.fill();

  // Clip to the circle plus the column above it, so the person rises out of
  // the circle's top without spilling past its sides.
  context.save();
  context.beginPath();
  context.arc(circleX, circleY, circleRadius, 0, Math.PI * 2);
  context.rect(
    circleX - circleRadius,
    0,
    circleRadius * 2,
    Math.max(0, circleY),
  );
  context.clip();
  drawPersonWithOutline(
    context,
    person,
    composition,
    personX,
    personY,
    personSize,
  );
  context.restore();
}

function drawPersonWithOutline(
  context: CanvasRenderingContext2D,
  person: CanvasImageSource,
  composition: AvatarComposition,
  personX: number,
  personY: number,
  personSize: number,
): void {
  if (composition.personOutline) {
    drawPersonOutline(
      context,
      person,
      composition.backdropColor,
      personX,
      personY,
      personSize,
    );
  }

  context.drawImage(person, personX, personY, personSize, personSize);
}

function drawPersonOutline(
  context: CanvasRenderingContext2D,
  person: CanvasImageSource,
  backdropColor: string | null,
  personX: number,
  personY: number,
  personSize: number,
): void {
  const outline = document.createElement("canvas");
  outline.width = ANIMATED_AVATAR_SIZE;
  outline.height = ANIMATED_AVATAR_SIZE;
  const outlineContext = outline.getContext("2d");
  if (!outlineContext) {
    return;
  }

  outlineContext.drawImage(person, personX, personY, personSize, personSize);
  outlineContext.globalCompositeOperation = "source-in";
  outlineContext.fillStyle = personOutlineColor(backdropColor);
  outlineContext.fillRect(0, 0, ANIMATED_AVATAR_SIZE, ANIMATED_AVATAR_SIZE);

  context.save();
  context.globalAlpha = PERSON_OUTLINE_ALPHA;
  context.imageSmoothingEnabled = false;
  for (const [offsetX, offsetY] of PERSON_OUTLINE_OFFSETS) {
    context.drawImage(outline, offsetX, offsetY);
  }
  context.restore();
}

function personOutlineColor(backdropColor: string | null): string {
  const color = backdropColor ? parseHexColor(backdropColor) : null;
  if (!color) {
    return "#ffffff";
  }

  const luma =
    (0.2126 * color.red + 0.7152 * color.green + 0.0722 * color.blue) / 255;
  return luma > 0.74 ? "#111111" : "#ffffff";
}

function parseHexColor(
  value: string,
): { red: number; green: number; blue: number } | null {
  const match = /^#?([0-9a-f]{6})$/i.exec(value);
  if (!match) {
    return null;
  }

  const hex = match[1];
  if (!hex) {
    return null;
  }

  return {
    blue: Number.parseInt(hex.slice(4, 6), 16),
    green: Number.parseInt(hex.slice(2, 4), 16),
    red: Number.parseInt(hex.slice(0, 2), 16),
  };
}

/** Compose every recorded frame with the chosen backdrop and framing. */
export function composeAvatarFrames(
  bitmaps: ImageBitmap[],
  composition: AvatarComposition,
): ImageData[] {
  const canvas = document.createElement("canvas");
  canvas.width = ANIMATED_AVATAR_SIZE;
  canvas.height = ANIMATED_AVATAR_SIZE;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) {
    throw new Error("Could not create a drawing context.");
  }

  return bitmaps.map((bitmap) => {
    composeAvatarFrame(context, bitmap, composition);
    return context.getImageData(
      0,
      0,
      ANIMATED_AVATAR_SIZE,
      ANIMATED_AVATAR_SIZE,
    );
  });
}

/** Mirror the sequence so the animation plays forward, then backward. */
export function buildPingPongAvatarFrames<T>(frames: T[]): T[] {
  if (frames.length <= 2) {
    return frames;
  }

  return frames.concat(frames.slice(1, -1).reverse());
}

/** Encode composed frames as an infinitely looping ping-pong animated PNG. */
export function encodeAvatarAnimation(frames: ImageData[]): Uint8Array {
  const animationFrames = buildPingPongAvatarFrames(frames);
  const buffers = animationFrames.map(
    (frame) =>
      frame.data.buffer.slice(
        frame.data.byteOffset,
        frame.data.byteOffset + frame.data.byteLength,
      ) as ArrayBuffer,
  );
  const delays = animationFrames.map(() => ANIMATED_AVATAR_FRAME_DELAY_MS);
  const encoded = UPNG.encode(
    buffers,
    ANIMATED_AVATAR_SIZE,
    ANIMATED_AVATAR_SIZE,
    APNG_COLOR_COUNT,
    delays,
  );
  return new Uint8Array(encoded);
}

function sampleMaskConfidence(
  values: Float32Array,
  width: number,
  height: number,
  x: number,
  y: number,
): number {
  const sampleX = Math.min(Math.max(x, 0), Math.max(0, width - 1));
  const sampleY = Math.min(Math.max(y, 0), Math.max(0, height - 1));
  const x0 = Math.floor(sampleX);
  const y0 = Math.floor(sampleY);
  const x1 = Math.min(width - 1, x0 + 1);
  const y1 = Math.min(height - 1, y0 + 1);
  const tx = sampleX - x0;
  const ty = sampleY - y0;
  const topLeft = values[y0 * width + x0] ?? 0;
  const topRight = values[y0 * width + x1] ?? topLeft;
  const bottomLeft = values[y1 * width + x0] ?? topLeft;
  const bottomRight = values[y1 * width + x1] ?? bottomLeft;
  const top = topLeft + (topRight - topLeft) * tx;
  const bottom = bottomLeft + (bottomRight - bottomLeft) * tx;
  return top + (bottom - top) * ty;
}

export async function renderAvatarPosterPng(
  frame: ImageData,
): Promise<Uint8Array> {
  const blob = await new Promise<Blob | null>((resolve) => {
    frameToCanvas(frame).toBlob(resolve, "image/png");
  });
  if (!blob) {
    throw new Error("Could not render the poster frame.");
  }
  return new Uint8Array(await blob.arrayBuffer());
}

function frameToCanvas(frame: ImageData): HTMLCanvasElement {
  const canvas = document.createElement("canvas");
  canvas.width = frame.width;
  canvas.height = frame.height;
  const context = canvas.getContext("2d");
  if (!context) {
    throw new Error("Could not create a drawing context.");
  }
  context.putImageData(frame, 0, 0);
  return canvas;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
