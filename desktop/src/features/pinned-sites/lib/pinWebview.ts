import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type PinWebviewBounds = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type PinWebviewNavState = {
  canGoBack: boolean;
  canGoForward: boolean;
  currentUrl: string;
};

export type PinWebviewPollResult = {
  changed: boolean;
};

export type PinWebviewLoadState = {
  pinId: string;
  url: string;
  ok: boolean;
  status?: number | null;
  message?: string | null;
};

/** Logical px. A 1×1 first layout must not create the child webview. */
export const MIN_PIN_WEBVIEW_EDGE = 32;

/** Playground overlay parks pins; they listen for this to show again. */
export const PIN_WEBVIEW_RESTORE_EVENT = "buzz:pin-webview-restore";

const EMPTY_NAV: PinWebviewNavState = {
  canGoBack: false,
  canGoForward: false,
  currentUrl: "",
};

export function pinWebviewBoundsAreUsable(bounds: PinWebviewBounds): boolean {
  return (
    bounds.width >= MIN_PIN_WEBVIEW_EDGE &&
    bounds.height >= MIN_PIN_WEBVIEW_EDGE
  );
}

function isNativePinRuntime(): boolean {
  return isTauri() || import.meta.env.MODE === "e2e";
}

async function invokePin<T>(
  command: string,
  args: Record<string, unknown>,
  fallback: T,
): Promise<T> {
  if (!isNativePinRuntime()) {
    return fallback;
  }
  return invoke<T>(command, args);
}

/**
 * Bumped by hide/hide-all/close. An in-flight show that started before a
 * dismiss must re-hide after resolve so it cannot resurrect an orphan
 * topmost WKWebView (AppShell leave-pin, playground park, embed open).
 * Re-hides from a stale show must NOT bump again — that would invalidate a
 * remounted pin's legitimate follow-up show.
 */
let pinHideEpoch = 0;

/**
 * Per-pin show generation. A remount (Strict Mode, url change, restore)
 * starts a newer show while an older one is still in flight. The older
 * show must not re-hide after the newer show has already painted — that
 * left the first open blank until a second click.
 */
const pinShowGeneration = new Map<string, number>();

export function bumpPinHideEpoch(): void {
  pinHideEpoch += 1;
}

/** Test helper. */
export function getPinHideEpoch(): number {
  return pinHideEpoch;
}

/** Test helper. */
export function getPinShowGeneration(pinId: string): number {
  return pinShowGeneration.get(pinId) ?? 0;
}

function nextPinShowGeneration(pinId: string): number {
  const next = (pinShowGeneration.get(pinId) ?? 0) + 1;
  pinShowGeneration.set(pinId, next);
  return next;
}

function isCurrentPinShow(pinId: string, generation: number): boolean {
  return pinShowGeneration.get(pinId) === generation;
}

async function invokeHidePinWebview(pinId: string): Promise<void> {
  await invokePin("pin_webview_hide", { pinId }, undefined);
}

export async function showPinWebview(input: {
  pinId: string;
  startUrl: string;
  bounds: PinWebviewBounds;
}): Promise<PinWebviewNavState> {
  const epoch = pinHideEpoch;
  const generation = nextPinShowGeneration(input.pinId);
  const nav = await invokePin(
    "pin_webview_show",
    {
      pinId: input.pinId,
      startUrl: input.startUrl,
      bounds: input.bounds,
    },
    {
      ...EMPTY_NAV,
      currentUrl: input.startUrl,
    },
  );
  // Dismiss won while we were in flight, and no newer show for this pin
  // took over — re-hide so we cannot orphan a topmost WKWebView. If a
  // remount already started another show, leave that paint alone.
  if (epoch !== pinHideEpoch && isCurrentPinShow(input.pinId, generation)) {
    await invokeHidePinWebview(input.pinId);
  }
  return nav;
}

export async function hidePinWebview(pinId: string): Promise<void> {
  bumpPinHideEpoch();
  await invokeHidePinWebview(pinId);
}

export async function hideAllPinWebviews(): Promise<void> {
  bumpPinHideEpoch();
  if (!isNativePinRuntime()) return;
  await invoke("pin_webview_hide_all");
}

export async function setPinWebviewBounds(
  pinId: string,
  bounds: PinWebviewBounds,
): Promise<void> {
  await invokePin("pin_webview_set_bounds", { pinId, bounds }, undefined);
}

export async function pinWebviewGoBack(
  pinId: string,
): Promise<PinWebviewNavState> {
  return invokePin("pin_webview_go_back", { pinId }, EMPTY_NAV);
}

export async function pinWebviewGoForward(
  pinId: string,
): Promise<PinWebviewNavState> {
  return invokePin("pin_webview_go_forward", { pinId }, EMPTY_NAV);
}

export async function pinWebviewReload(pinId: string): Promise<void> {
  await invokePin("pin_webview_reload", { pinId }, undefined);
}

export async function getPinWebviewNavState(
  pinId: string,
): Promise<PinWebviewNavState> {
  try {
    return await invokePin("pin_webview_nav_state", { pinId }, EMPTY_NAV);
  } catch {
    return EMPTY_NAV;
  }
}

export async function pollPinWebview(
  pinId: string,
  startUrl: string,
): Promise<PinWebviewPollResult> {
  return invokePin("pin_webview_poll", { pinId, startUrl }, { changed: false });
}

export async function closePinWebview(pinId: string): Promise<void> {
  bumpPinHideEpoch();
  if (!isNativePinRuntime()) return;
  await invoke("pin_webview_close", { pinId });
}

export function subscribePinWebviewNav(
  onNav: (state: PinWebviewNavState & { pinId: string }) => void,
): Promise<() => void> {
  if (!isNativePinRuntime()) {
    return Promise.resolve(() => {});
  }
  return listen<PinWebviewNavState & { pinId: string }>(
    "pin-webview-nav",
    (event) => {
      onNav(event.payload);
    },
  ).catch(() => () => {});
}

export function subscribePinWebviewLoad(
  onLoad: (state: PinWebviewLoadState) => void,
): Promise<() => void> {
  if (!isNativePinRuntime()) {
    return Promise.resolve(() => {});
  }
  return listen<PinWebviewLoadState>("pin-webview-load", (event) => {
    onLoad(event.payload);
  }).catch(() => () => {});
}
