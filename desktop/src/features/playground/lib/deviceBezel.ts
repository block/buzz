import { type PlaygroundDevice, playgroundDeviceViewport } from "./devices.ts";

export type PlaygroundBezelChrome =
  | "iphone-island"
  | "iphone-home-button"
  | "pixel-punch"
  | "ipad";

export type PlaygroundBezelEdge = "top" | "right" | "bottom" | "left";

export type PlaygroundBezelPadding = {
  top: number;
  right: number;
  bottom: number;
  left: number;
};

export type PlaygroundBezelNub = {
  id: string;
  edge: PlaygroundBezelEdge;
  /** Position along the edge, 0–1 from the portrait-top / landscape-left origin. */
  offsetRatio: number;
  length: number;
  thickness: number;
};

export type PlaygroundDeviceBezel = {
  family: PlaygroundDevice["family"];
  chrome: PlaygroundBezelChrome;
  /** Outer hardware corner radius. Phones are ≥ 3rem (48px) at a 16px rem. */
  outerRadius: number;
  /** Inner screen-hole radius. The bezel paints over square page corners. */
  innerRadius: number;
  padding: PlaygroundBezelPadding;
  bezelColor: string;
  nubColor: string;
  faceColor: string;
  indicatorColor: string;
  nubs: readonly PlaygroundBezelNub[];
  island?: { width: number; height: number };
  punch?: { width: number; height: number };
  homeIndicator?: { width: number; height: number };
  homeButton?: { size: number };
  cameraDot?: { size: number };
};

export const PLAYGROUND_PHONE_BEZEL_COLOR = "#1a1a1c";
export const PLAYGROUND_TABLET_BEZEL_COLOR = "#242426";
export const PLAYGROUND_BEZEL_NUB_COLOR = "#2e2e32";
export const PLAYGROUND_BEZEL_FACE_COLOR = "#050506";
export const PLAYGROUND_BEZEL_INDICATOR_COLOR = "#d4d4d8";

const IPHONE_NUBS: readonly PlaygroundBezelNub[] = [
  { id: "action", edge: "left", offsetRatio: 0.16, length: 18, thickness: 3 },
  {
    id: "volume-up",
    edge: "left",
    offsetRatio: 0.22,
    length: 28,
    thickness: 3,
  },
  {
    id: "volume-down",
    edge: "left",
    offsetRatio: 0.3,
    length: 28,
    thickness: 3,
  },
  { id: "power", edge: "right", offsetRatio: 0.22, length: 52, thickness: 3 },
];

const PIXEL_NUBS: readonly PlaygroundBezelNub[] = [
  { id: "power", edge: "right", offsetRatio: 0.18, length: 48, thickness: 3 },
  {
    id: "volume-up",
    edge: "right",
    offsetRatio: 0.28,
    length: 36,
    thickness: 3,
  },
  {
    id: "volume-down",
    edge: "right",
    offsetRatio: 0.36,
    length: 36,
    thickness: 3,
  },
];

function phonePaint() {
  return {
    bezelColor: PLAYGROUND_PHONE_BEZEL_COLOR,
    nubColor: PLAYGROUND_BEZEL_NUB_COLOR,
    faceColor: PLAYGROUND_BEZEL_FACE_COLOR,
    indicatorColor: PLAYGROUND_BEZEL_INDICATOR_COLOR,
  };
}

function tabletPaint() {
  return {
    bezelColor: PLAYGROUND_TABLET_BEZEL_COLOR,
    nubColor: PLAYGROUND_BEZEL_NUB_COLOR,
    faceColor: PLAYGROUND_BEZEL_FACE_COLOR,
    indicatorColor: PLAYGROUND_BEZEL_INDICATOR_COLOR,
  };
}

function portraitBezel(device: PlaygroundDevice): PlaygroundDeviceBezel {
  switch (device.id) {
    case "iphone-se":
      return {
        family: "iphone",
        chrome: "iphone-home-button",
        outerRadius: 48,
        innerRadius: 12,
        padding: { top: 50, right: 16, bottom: 68, left: 16 },
        ...phonePaint(),
        nubs: IPHONE_NUBS,
        homeButton: { size: 44 },
      };
    case "iphone-16-pro-max":
      return {
        family: "iphone",
        chrome: "iphone-island",
        outerRadius: 56,
        innerRadius: 40,
        padding: { top: 40, right: 16, bottom: 32, left: 16 },
        ...phonePaint(),
        nubs: IPHONE_NUBS,
        island: { width: 134, height: 36 },
        homeIndicator: { width: 140, height: 5 },
      };
    case "iphone-16":
      return {
        family: "iphone",
        chrome: "iphone-island",
        outerRadius: 52,
        innerRadius: 36,
        padding: { top: 38, right: 16, bottom: 30, left: 16 },
        ...phonePaint(),
        nubs: IPHONE_NUBS,
        island: { width: 126, height: 34 },
        homeIndicator: { width: 128, height: 5 },
      };
    case "pixel-8":
      return {
        family: "pixel",
        chrome: "pixel-punch",
        outerRadius: 48,
        innerRadius: 32,
        padding: { top: 34, right: 14, bottom: 26, left: 14 },
        ...phonePaint(),
        nubs: PIXEL_NUBS,
        punch: { width: 72, height: 20 },
        homeIndicator: { width: 112, height: 5 },
      };
    case "ipad-mini":
    case "ipad-pro-11":
      return {
        family: "ipad",
        chrome: "ipad",
        outerRadius: 40,
        innerRadius: 22,
        padding: { top: 14, right: 14, bottom: 14, left: 14 },
        ...tabletPaint(),
        nubs: [],
        cameraDot: { size: 8 },
      };
  }
}

/** Rotate portrait chrome 90° CCW so the island / chin follow the long edge. */
export function rotateBezelEdge(
  edge: PlaygroundBezelEdge,
): PlaygroundBezelEdge {
  switch (edge) {
    case "top":
      return "left";
    case "left":
      return "bottom";
    case "bottom":
      return "right";
    case "right":
      return "top";
  }
}

function rotatePadding(
  padding: PlaygroundBezelPadding,
): PlaygroundBezelPadding {
  return {
    top: padding.right,
    right: padding.bottom,
    bottom: padding.left,
    left: padding.top,
  };
}

/**
 * Hardware chrome for a museum device. Padding is *outside* the published
 * CSS viewport so the native WKWebView can stay the screen hole.
 */
export function playgroundDeviceBezel(
  device: PlaygroundDevice,
  orientation: "portrait" | "landscape",
): PlaygroundDeviceBezel {
  const portrait = portraitBezel(device);
  if (orientation === "portrait") {
    return portrait;
  }
  return {
    ...portrait,
    padding: rotatePadding(portrait.padding),
    nubs: portrait.nubs.map((nub) => ({
      ...nub,
      edge: rotateBezelEdge(nub.edge),
    })),
  };
}

export function playgroundDeviceBezelOuterSize(
  viewport: { width: number; height: number },
  bezel: PlaygroundDeviceBezel,
): { width: number; height: number } {
  return {
    width: viewport.width + bezel.padding.left + bezel.padding.right,
    height: viewport.height + bezel.padding.top + bezel.padding.bottom,
  };
}

export function playgroundDeviceFrameSize(
  device: PlaygroundDevice,
  orientation: "portrait" | "landscape",
): { width: number; height: number } {
  const viewport = playgroundDeviceViewport(device, orientation);
  return playgroundDeviceBezelOuterSize(
    viewport,
    playgroundDeviceBezel(device, orientation),
  );
}

/** Side-button nubs sit outside the frame; keep them inside the scroller. */
export function playgroundDeviceNubGutter(
  bezel: PlaygroundDeviceBezel,
): number {
  return bezel.nubs.reduce((max, nub) => Math.max(max, nub.thickness), 0);
}

/**
 * Native WKWebView bounds come from the inner screen host, never the outer
 * bezel box. `viewport` is the published CSS size when the host may not have
 * finished layout.
 *
 * Native WKWebView always paints on top of HTML. Clamp y to the bottom of
 * playground chrome so Desktop/Responsive/Mobile cannot sit under the page.
 * Desktop fills the remaining host; mobile/responsive keep published viewport.
 */
export function readPlaygroundStageBounds(
  el: HTMLElement,
  viewport?: { width: number; height: number },
  chrome?: Element | null,
): { x: number; y: number; width: number; height: number } {
  const rect = el.getBoundingClientRect();
  const chromeEl =
    chrome ??
    el
      .closest?.('[data-testid="playground-overlay"]')
      ?.querySelector('[data-testid="playground-chrome"]');
  const chromeBottom = chromeEl?.getBoundingClientRect().bottom;
  const y =
    typeof chromeBottom === "number" ? Math.max(rect.y, chromeBottom) : rect.y;
  const bottom =
    typeof rect.bottom === "number" ? rect.bottom : rect.y + rect.height;
  return {
    x: rect.x,
    y,
    width: viewport?.width ?? rect.width,
    height: viewport?.height ?? Math.max(0, bottom - y),
  };
}
