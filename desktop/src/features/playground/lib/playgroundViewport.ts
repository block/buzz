/**
 * Per-session playground viewport snapshot for Browsers list captions and
 * restoring Desktop/Responsive/Mobile when chrome remounts.
 * In-memory only (not persisted). Agent Drive MCP can patch via
 * browser_set_viewport → host emit → applyAgentPlaygroundViewport.
 */

import {
  PLAYGROUND_DEVICES,
  PLAYGROUND_DEVICE_SCALE_DEFAULT,
  clampPlaygroundDeviceScale,
  getPlaygroundDevice,
  playgroundDeviceViewport,
  scalePlaygroundDeviceViewport,
  type PlaygroundDeviceId,
  type PlaygroundDeviceScalePercent,
} from "./devices.ts";
import { DEFAULT_RESPONSIVE_VIEWPORT } from "./types.ts";

export type PlaygroundChromeMode = "desktop" | "responsive" | "mobile";

export type PlaygroundViewportOrientation = "portrait" | "landscape";

export type PlaygroundViewportSnapshot = {
  mode: PlaygroundChromeMode;
  /** CSS viewport width; 0 = unknown / not measured yet. */
  width: number;
  /** CSS viewport height; 0 = unknown / not measured yet. */
  height: number;
  /** Mobile museum scale percent (50–200). Omitted for desktop/responsive. */
  scalePercent?: number;
  /** Mobile museum device id. */
  deviceId?: PlaygroundDeviceId;
  /** Mobile museum orientation. */
  orientation?: PlaygroundViewportOrientation;
};

const DEFAULT_SNAPSHOT: PlaygroundViewportSnapshot = {
  mode: "desktop",
  width: 0,
  height: 0,
};

const bySid = new Map<string, PlaygroundViewportSnapshot>();
const listeners = new Set<() => void>();
let revision = 0;

function emit() {
  revision += 1;
  for (const listener of listeners) listener();
}

function sameSnapshot(
  a: PlaygroundViewportSnapshot,
  b: PlaygroundViewportSnapshot,
): boolean {
  return (
    a.mode === b.mode &&
    a.width === b.width &&
    a.height === b.height &&
    (a.scalePercent ?? null) === (b.scalePercent ?? null) &&
    (a.deviceId ?? null) === (b.deviceId ?? null) &&
    (a.orientation ?? null) === (b.orientation ?? null)
  );
}

export function subscribePlaygroundViewport(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getPlaygroundViewportRevision(): number {
  return revision;
}

export function getPlaygroundViewport(
  sid: string,
): PlaygroundViewportSnapshot {
  return bySid.get(sid) ?? DEFAULT_SNAPSHOT;
}

/** Patch/merge viewport fields for a session; no-op when unchanged. */
export function setPlaygroundViewport(
  sid: string,
  patch: Partial<PlaygroundViewportSnapshot> &
    Pick<PlaygroundViewportSnapshot, "mode">,
): PlaygroundViewportSnapshot {
  const prev = bySid.get(sid) ?? DEFAULT_SNAPSHOT;
  const next: PlaygroundViewportSnapshot = {
    mode: patch.mode,
    width: patch.width ?? prev.width,
    height: patch.height ?? prev.height,
  };
  if (patch.mode === "mobile") {
    const scale =
      patch.scalePercent ?? prev.scalePercent ?? PLAYGROUND_DEVICE_SCALE_DEFAULT;
    next.scalePercent = scale;
    next.deviceId = patch.deviceId ?? prev.deviceId ?? "iphone-16";
    next.orientation = patch.orientation ?? prev.orientation ?? "portrait";
    // Prefer explicit width/height (agent snapshot); else derive from device.
    if (patch.width == null || patch.height == null) {
      const device = getPlaygroundDevice(next.deviceId);
      const base = device
        ? playgroundDeviceViewport(device, next.orientation)
        : { width: 393, height: 852 };
      const scaled = scalePlaygroundDeviceViewport(base, scale);
      next.width = patch.width ?? scaled.width;
      next.height = patch.height ?? scaled.height;
    }
  }
  if (sameSnapshot(prev, next) && bySid.has(sid)) {
    return prev;
  }
  bySid.set(sid, next);
  emit();
  return next;
}

export function clearPlaygroundViewport(sid: string): void {
  if (!bySid.delete(sid)) return;
  emit();
}

export function resetPlaygroundViewports(): void {
  if (bySid.size === 0) return;
  bySid.clear();
  emit();
}

const MODE_LABEL: Record<PlaygroundChromeMode, string> = {
  desktop: "Desktop",
  responsive: "Responsive",
  mobile: "Mobile",
};

/** Compact list chip: `Desktop · 1280×800` / `Mobile · 393×852 · 150%`. */
export function playgroundViewportCaption(
  snapshot: PlaygroundViewportSnapshot | null | undefined,
): string {
  const s = snapshot ?? DEFAULT_SNAPSHOT;
  const label = MODE_LABEL[s.mode] ?? "Desktop";
  const hasDims = s.width > 0 && s.height > 0;
  const dims = hasDims ? `${s.width}×${s.height}` : null;
  const scale =
    s.mode === "mobile" &&
    s.scalePercent != null &&
    s.scalePercent !== 100
      ? `${s.scalePercent}%`
      : null;
  return [label, dims, scale].filter(Boolean).join(" · ");
}

const DEVICE_IDS: ReadonlySet<string> = new Set(
  PLAYGROUND_DEVICES.map((d) => d.id),
);

export type AgentViewportRequest = {
  mode: PlaygroundChromeMode;
  width?: number;
  height?: number;
  deviceId?: PlaygroundDeviceId;
  orientation?: PlaygroundViewportOrientation;
  scalePercent?: number;
};

export type AgentViewportNormalizeResult =
  | { ok: true; value: AgentViewportRequest; snapshot: PlaygroundViewportSnapshot }
  | { ok: false; error: string };

function asFinitePositiveInt(value: unknown, field: string): number | string {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return `${field} must be a finite number`;
  }
  const n = Math.round(value);
  if (n < 1) return `${field} must be >= 1`;
  return n;
}

/**
 * Validate agent / MCP browser_set_viewport payload and compute the store
 * snapshot Desktop should apply (mirrors Stage UI setters).
 */
export function normalizeAgentViewportRequest(
  raw: unknown,
): AgentViewportNormalizeResult {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    return { ok: false, error: "viewport request must be an object" };
  }
  const candidate = raw as Record<string, unknown>;
  const modeRaw =
    typeof candidate.mode === "string" ? candidate.mode.trim().toLowerCase() : "";
  if (modeRaw !== "desktop" && modeRaw !== "responsive" && modeRaw !== "mobile") {
    return {
      ok: false,
      error: 'mode must be "desktop" | "responsive" | "mobile"',
    };
  }
  const mode = modeRaw as PlaygroundChromeMode;

  if (mode === "desktop") {
    const value: AgentViewportRequest = { mode: "desktop" };
    return {
      ok: true,
      value,
      snapshot: { mode: "desktop", width: 0, height: 0 },
    };
  }

  if (mode === "responsive") {
    const widthRaw =
      candidate.width !== undefined
        ? asFinitePositiveInt(candidate.width, "width")
        : DEFAULT_RESPONSIVE_VIEWPORT.width;
    if (typeof widthRaw === "string") return { ok: false, error: widthRaw };
    const heightRaw =
      candidate.height !== undefined
        ? asFinitePositiveInt(candidate.height, "height")
        : DEFAULT_RESPONSIVE_VIEWPORT.height;
    if (typeof heightRaw === "string") return { ok: false, error: heightRaw };
    const width = Math.max(320, widthRaw);
    const height = Math.max(320, heightRaw);
    const value: AgentViewportRequest = { mode: "responsive", width, height };
    return {
      ok: true,
      value,
      snapshot: { mode: "responsive", width, height },
    };
  }

  // mobile
  let deviceId: PlaygroundDeviceId = "iphone-16";
  if (candidate.deviceId !== undefined) {
    if (
      typeof candidate.deviceId !== "string" ||
      !DEVICE_IDS.has(candidate.deviceId.trim())
    ) {
      return {
        ok: false,
        error: `deviceId must be one of: ${[...DEVICE_IDS].join(", ")}`,
      };
    }
    deviceId = candidate.deviceId.trim() as PlaygroundDeviceId;
  }
  let orientation: PlaygroundViewportOrientation = "portrait";
  if (candidate.orientation !== undefined) {
    if (
      candidate.orientation !== "portrait" &&
      candidate.orientation !== "landscape"
    ) {
      return {
        ok: false,
        error: 'orientation must be "portrait" | "landscape"',
      };
    }
    orientation = candidate.orientation;
  }
  let scalePercent: PlaygroundDeviceScalePercent =
    PLAYGROUND_DEVICE_SCALE_DEFAULT;
  if (candidate.scalePercent !== undefined) {
    if (
      typeof candidate.scalePercent !== "number" ||
      !Number.isFinite(candidate.scalePercent)
    ) {
      return { ok: false, error: "scalePercent must be a finite number" };
    }
    scalePercent = clampPlaygroundDeviceScale(candidate.scalePercent);
  }
  const device = getPlaygroundDevice(deviceId);
  const base = device
    ? playgroundDeviceViewport(device, orientation)
    : { width: 393, height: 852 };
  const scaled = scalePlaygroundDeviceViewport(base, scalePercent);
  const value: AgentViewportRequest = {
    mode: "mobile",
    deviceId,
    orientation,
    scalePercent,
    width: scaled.width,
    height: scaled.height,
  };
  return {
    ok: true,
    value,
    snapshot: {
      mode: "mobile",
      width: scaled.width,
      height: scaled.height,
      scalePercent,
      deviceId,
      orientation,
    },
  };
}

/** Apply a validated agent viewport request into the in-memory store. */
export function applyAgentPlaygroundViewport(
  sid: string,
  raw: unknown,
): AgentViewportNormalizeResult {
  const normalized = normalizeAgentViewportRequest(raw);
  if (!normalized.ok) return normalized;
  const applied = setPlaygroundViewport(sid, normalized.snapshot);
  return { ok: true, value: normalized.value, snapshot: applied };
}
