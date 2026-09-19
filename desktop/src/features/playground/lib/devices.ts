export type PlaygroundDeviceId =
  | "iphone-se"
  | "iphone-16"
  | "iphone-16-pro-max"
  | "pixel-8"
  | "ipad-mini"
  | "ipad-pro-11";

export type PlaygroundDevice = {
  id: PlaygroundDeviceId;
  name: string;
  family: "iphone" | "pixel" | "ipad";
  /** CSS viewport in portrait. */
  width: number;
  height: number;
  userAgent: string;
};

/** Safari-like desktop UA. Desktop and Responsive keep this. */
export const PLAYGROUND_DESKTOP_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";

const IPHONE_UA =
  "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";
const PIXEL_UA =
  "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Mobile Safari/537.36";
const IPAD_UA =
  "Mozilla/5.0 (iPad; CPU OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";

/**
 * Public CSS viewports (not hardware pixels), verified against current
 * published device lists (viewpo.io, screensize.io, Apple / Pixel CSS refs):
 * - iPhone SE (3rd gen): 375×667
 * - iPhone 16: 393×852
 * - iPhone 16 Pro Max: 440×956
 * - Pixel 8: 412×915
 * - iPad mini (6/7 / A17 Pro): 744×1133
 * - iPad Pro 11" (M4, 2420×1668 @2×): 834×1210
 */
export const PLAYGROUND_DEVICES: readonly PlaygroundDevice[] = [
  {
    id: "iphone-se",
    name: "iPhone SE",
    family: "iphone",
    width: 375,
    height: 667,
    userAgent: IPHONE_UA,
  },
  {
    id: "iphone-16",
    name: "iPhone 16",
    family: "iphone",
    width: 393,
    height: 852,
    userAgent: IPHONE_UA,
  },
  {
    id: "iphone-16-pro-max",
    name: "iPhone 16 Pro Max",
    family: "iphone",
    width: 440,
    height: 956,
    userAgent: IPHONE_UA,
  },
  {
    id: "pixel-8",
    name: "Pixel 8",
    family: "pixel",
    width: 412,
    height: 915,
    userAgent: PIXEL_UA,
  },
  {
    id: "ipad-mini",
    name: "iPad mini",
    family: "ipad",
    width: 744,
    height: 1133,
    userAgent: IPAD_UA,
  },
  {
    id: "ipad-pro-11",
    name: "iPad Pro 11",
    family: "ipad",
    width: 834,
    height: 1210,
    userAgent: IPAD_UA,
  },
];

export function playgroundUserAgent(
  mode: "desktop" | "responsive" | "mobile",
  device?: PlaygroundDevice,
): string {
  if (mode === "mobile" && device) {
    return device.userAgent;
  }
  return PLAYGROUND_DESKTOP_UA;
}

export function getPlaygroundDevice(
  id: PlaygroundDeviceId,
): PlaygroundDevice | undefined {
  return PLAYGROUND_DEVICES.find((device) => device.id === id);
}

export function playgroundDeviceViewport(
  device: PlaygroundDevice,
  orientation: "portrait" | "landscape",
): { width: number; height: number } {
  if (orientation === "landscape") {
    return { width: device.height, height: device.width };
  }
  return { width: device.width, height: device.height };
}

/** Mobile museum visual scale (percent of published CSS viewport). */
export const PLAYGROUND_DEVICE_SCALE_MIN = 50;
export const PLAYGROUND_DEVICE_SCALE_MAX = 200;
export const PLAYGROUND_DEVICE_SCALE_STEP = 25;
export const PLAYGROUND_DEVICE_SCALE_DEFAULT = 100;

export type PlaygroundDeviceScalePercent =
  | 50
  | 75
  | 100
  | 125
  | 150
  | 175
  | 200;

export function clampPlaygroundDeviceScale(
  percent: number,
): PlaygroundDeviceScalePercent {
  const stepped =
    Math.round(percent / PLAYGROUND_DEVICE_SCALE_STEP) *
    PLAYGROUND_DEVICE_SCALE_STEP;
  const clamped = Math.min(
    PLAYGROUND_DEVICE_SCALE_MAX,
    Math.max(PLAYGROUND_DEVICE_SCALE_MIN, stepped),
  );
  return clamped as PlaygroundDeviceScalePercent;
}

export function stepPlaygroundDeviceScale(
  percent: number,
  direction: -1 | 1,
): PlaygroundDeviceScalePercent {
  return clampPlaygroundDeviceScale(
    percent + direction * PLAYGROUND_DEVICE_SCALE_STEP,
  );
}

export function playgroundDeviceScaleFactor(percent: number): number {
  return clampPlaygroundDeviceScale(percent) / 100;
}

/** Scale a published CSS viewport (or any W×H) by museum scale percent. */
export function scalePlaygroundDeviceViewport(
  viewport: { width: number; height: number },
  scalePercent: number,
): { width: number; height: number } {
  const factor = playgroundDeviceScaleFactor(scalePercent);
  return {
    width: Math.max(1, Math.round(viewport.width * factor)),
    height: Math.max(1, Math.round(viewport.height * factor)),
  };
}
