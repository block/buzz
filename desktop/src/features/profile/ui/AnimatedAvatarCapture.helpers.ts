import {
  ANIMATED_AVATAR_DURATION_MS,
  ANIMATED_AVATAR_SIZE,
  type AvatarCameraDevice,
  type AvatarComposition,
  composeAvatarFrame,
  DEFAULT_PERSON_SCALE,
} from "@/features/profile/lib/animatedAvatarCapture";
import { AVATAR_COLORS } from "@/features/profile/ui/ProfileAvatarEditor.utils";

export type CapturePhase =
  | "idle"
  | "starting"
  | "live"
  | "recording"
  | "processing"
  | "review";
export type CameraSource = "computer" | "iphone";

const FILMSTRIP_FRAME_SIZE = 48;
const SLIDER_TICK_STEP = 10;
const PHONE_DEFAULT_PERSON_SCALE =
  (Math.round(DEFAULT_PERSON_SCALE * 100) - SLIDER_TICK_STEP) / 100;

export const RECORD_SECONDS = ANIMATED_AVATAR_DURATION_MS / 1000;
export const PERSON_SIZE_TIP =
  "Scale just past the color circle for a pop-out look.";
export const ENTRANCE_TRANSITION = {
  duration: 0.18,
  ease: [0.23, 1, 0.32, 1],
} as const;

/** Keep things draggable but never fully lost off-frame. */
const MAX_OFFSET = 192;

export function clampOffset(value: number): number {
  return Math.min(MAX_OFFSET, Math.max(-MAX_OFFSET, value));
}

export function randomBackdropColor(): string {
  return (
    AVATAR_COLORS[Math.floor(Math.random() * AVATAR_COLORS.length)] ??
    AVATAR_COLORS[0] ??
    "#FFFFFF"
  );
}

function isIPhoneCamera(device: AvatarCameraDevice): boolean {
  return /\b(continuity|ios|iphone|mobile|phone)\b/i.test(device.label);
}

function isFrontCamera(device: AvatarCameraDevice): boolean {
  return /\b(front|facetime|user)\b/i.test(device.label);
}

export function cameraLabelsAreAvailable(
  devices: AvatarCameraDevice[],
): boolean {
  return devices.some((device) => device.label.trim().length > 0);
}

export function preferredCameraDevice(
  devices: AvatarCameraDevice[],
  source: CameraSource,
): AvatarCameraDevice | null {
  const candidates =
    source === "iphone"
      ? devices.filter(isIPhoneCamera)
      : devices.filter((device) => !isIPhoneCamera(device));
  return (
    candidates.find(isFrontCamera) ??
    candidates[0] ??
    (source === "computer" ? devices[0] : null) ??
    null
  );
}

export function defaultPersonScaleForSource(
  source: CameraSource | null,
): number {
  return source === "iphone"
    ? PHONE_DEFAULT_PERSON_SCALE
    : DEFAULT_PERSON_SCALE;
}

export function clampFrameIndex(value: number, frameCount: number): number {
  return Math.min(Math.max(0, frameCount - 1), Math.max(0, value));
}

export function buildFilmstripFrames(
  bitmaps: ImageBitmap[],
  composition: AvatarComposition,
): string[] {
  const composed = document.createElement("canvas");
  composed.width = ANIMATED_AVATAR_SIZE;
  composed.height = ANIMATED_AVATAR_SIZE;
  const composedContext = composed.getContext("2d");
  const thumbnail = document.createElement("canvas");
  thumbnail.width = FILMSTRIP_FRAME_SIZE;
  thumbnail.height = FILMSTRIP_FRAME_SIZE;
  const thumbnailContext = thumbnail.getContext("2d");
  if (!composedContext || !thumbnailContext) {
    return [];
  }

  const urls: string[] = [];
  for (const bitmap of bitmaps) {
    composeAvatarFrame(composedContext, bitmap, composition);
    thumbnailContext.clearRect(
      0,
      0,
      FILMSTRIP_FRAME_SIZE,
      FILMSTRIP_FRAME_SIZE,
    );
    thumbnailContext.drawImage(
      composed,
      0,
      0,
      FILMSTRIP_FRAME_SIZE,
      FILMSTRIP_FRAME_SIZE,
    );
    urls.push(thumbnail.toDataURL("image/png"));
  }
  return urls;
}
