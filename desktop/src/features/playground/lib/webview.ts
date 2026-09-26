import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import {
  MIN_PLAYGROUND_WEBVIEW_EDGE,
  PLAYGROUND_APP_WEBVIEW_ID,
  PLAYGROUND_WEBVIEW_PREFIX,
  type PlaygroundInspectResult,
  type PlaygroundNavState,
  type PlaygroundPollResult,
  type PlaygroundScreenshotPayload,
  type PlaygroundWebviewBounds,
} from "./types.ts";

const EMPTY_NAV = {
  canGoBack: false,
  canGoForward: false,
  currentUrl: "",
};

function isNativePlaygroundRuntime(): boolean {
  return isTauri() || import.meta.env.MODE === "e2e";
}

export function currentWindowLabel(): string {
  if (!isTauri()) return "main";
  try {
    return getCurrentWindow().label || "main";
  } catch {
    return "main";
  }
}

function withWindowLabel(
  args: Record<string, unknown>,
): Record<string, unknown> {
  return { ...args, windowLabel: currentWindowLabel() };
}

async function invokePlayground<T>(
  command: string,
  args: Record<string, unknown>,
  fallback: T,
): Promise<T> {
  if (!isNativePlaygroundRuntime()) {
    return fallback;
  }
  return invoke<T>(command, withWindowLabel(args));
}

export function playgroundWebviewId(sid: string): string {
  return `${PLAYGROUND_WEBVIEW_PREFIX}${sid}`;
}

/**
 * Native child label for a playground on a specific window.
 * Main stays `playground-{sid}`; other windows use `playground-{sid}--{window}`.
 */
export function playgroundWebviewLabelForWindow(
  sid: string,
  windowLabel = "main",
): string {
  const cleaned = windowLabel.trim() || "main";
  if (cleaned === "main") return playgroundWebviewId(sid);
  return `${PLAYGROUND_WEBVIEW_PREFIX}${sid}--${cleaned}`;
}

/** True when this hide/close invoke targets the caller's current window. */
export function playgroundHideCloseIsWindowScoped(
  args: Record<string, unknown>,
  windowLabel: string,
): boolean {
  return args.windowLabel === windowLabel;
}

export function isPlaygroundInspectTarget(webviewId: string): boolean {
  return (
    webviewId.startsWith(PLAYGROUND_WEBVIEW_PREFIX) &&
    webviewId !== PLAYGROUND_APP_WEBVIEW_ID
  );
}

export function playgroundWebviewBoundsAreUsable(
  bounds: PlaygroundWebviewBounds,
): boolean {
  return (
    bounds.width >= MIN_PLAYGROUND_WEBVIEW_EDGE &&
    bounds.height >= MIN_PLAYGROUND_WEBVIEW_EDGE
  );
}

/**
 * Playground NativeStageHost listens for this after a modal park or a late
 * keeper `visible: false` show that stomped an on-stage paint.
 */
export const PLAYGROUND_WEBVIEW_RESTORE_EVENT =
  "buzz:playground-webview-restore";

/**
 * Bumped by visible shows. A parked keeper show (visible: false) that started
 * before a stage/embed show must not leave the WKWebView hidden off-screen —
 * that looked like "blank until Inspect" because Inspect re-applies bounds.
 */
const playgroundShowGeneration = new Map<string, number>();

/** Last on-stage / visible bounds per sid — keeper park must not invent 64×64. */
const playgroundLastVisibleBounds = new Map<string, PlaygroundWebviewBounds>();

export function getPlaygroundShowGeneration(sid: string): number {
  return playgroundShowGeneration.get(sid) ?? 0;
}

/** Test helper. */
export function resetPlaygroundShowGeneration(): void {
  playgroundShowGeneration.clear();
  playgroundLastVisibleBounds.clear();
}

/**
 * Bounds for a parked (visible:false) keeper show. Prefer the last full stage
 * size so Drive/Observe keep a real viewport while the React host is closed.
 * Falls back to the legacy off-screen 64×64 square only when never shown.
 */
export function playgroundKeeperParkBounds(
  sid: string,
  fallback: PlaygroundWebviewBounds = {
    x: -64,
    y: -64,
    width: 64,
    height: 64,
  },
): PlaygroundWebviewBounds {
  const last = playgroundLastVisibleBounds.get(sid);
  if (
    last &&
    last.width >= MIN_PLAYGROUND_WEBVIEW_EDGE &&
    last.height >= MIN_PLAYGROUND_WEBVIEW_EDGE &&
    (last.width > 64 || last.height > 64)
  ) {
    return {
      x: fallback.x,
      y: fallback.y,
      width: last.width,
      height: last.height,
    };
  }
  return { ...fallback };
}

export function getPlaygroundLastVisibleBounds(
  sid: string,
): PlaygroundWebviewBounds | undefined {
  return playgroundLastVisibleBounds.get(sid);
}

function nextPlaygroundShowGeneration(sid: string): number {
  const next = (playgroundShowGeneration.get(sid) ?? 0) + 1;
  playgroundShowGeneration.set(sid, next);
  return next;
}

function dispatchPlaygroundRestore(): void {
  if (typeof window === "undefined") return;
  if (typeof window.dispatchEvent !== "function") return;
  const EventCtor = window.Event;
  if (typeof EventCtor !== "function") return;
  try {
    window.dispatchEvent(new EventCtor(PLAYGROUND_WEBVIEW_RESTORE_EVENT));
  } catch {
    // Node test hosts may lack a DOM Event implementation.
  }
}

export async function showPlaygroundWebview(input: {
  sid: string;
  url: string;
  bounds: PlaygroundWebviewBounds;
  visible?: boolean;
  userAgent?: string;
}): Promise<PlaygroundNavState> {
  const wantVisible = input.visible ?? true;
  // Visible stage/embed claims ownership. Keeper (visible: false) snapshots
  // the current generation and must restore if a visible show won mid-flight.
  const generationAtStart = wantVisible
    ? nextPlaygroundShowGeneration(input.sid)
    : (playgroundShowGeneration.get(input.sid) ?? 0);
  if (wantVisible && playgroundWebviewBoundsAreUsable(input.bounds)) {
    playgroundLastVisibleBounds.set(input.sid, { ...input.bounds });
  }
  const nav = await invokePlayground(
    "playground_webview_show",
    {
      sid: input.sid,
      url: input.url,
      bounds: input.bounds,
      visible: wantVisible,
      userAgent: input.userAgent ?? null,
    },
    { sid: input.sid, ...EMPTY_NAV, currentUrl: input.url },
  );
  if (
    !wantVisible &&
    (playgroundShowGeneration.get(input.sid) ?? 0) !== generationAtStart
  ) {
    // Late keeper hide/offscreen bounds stomped an on-stage show — ask the
    // mounted host to paint again (same signal as modal-park release).
    dispatchPlaygroundRestore();
  }
  return nav;
}

export async function hidePlaygroundWebview(sid: string): Promise<void> {
  // Intentional park / unmount releases stage ownership so a concurrent
  // keeper does not dispatch restore into a tearing-down host.
  playgroundShowGeneration.delete(sid);
  await invokePlayground("playground_webview_hide", { sid }, undefined);
}

export async function hideAllPlaygroundWebviews(): Promise<void> {
  if (!isNativePlaygroundRuntime()) return;
  playgroundShowGeneration.clear();
  await invoke("playground_webview_hide_all", withWindowLabel({}));
}

export async function setPlaygroundWebviewBounds(
  sid: string,
  bounds: PlaygroundWebviewBounds,
  userAgent?: string,
): Promise<void> {
  await invokePlayground(
    "playground_webview_set_bounds",
    { sid, bounds, userAgent: userAgent ?? null },
    undefined,
  );
}

export async function closePlaygroundWebview(sid: string): Promise<void> {
  if (!isNativePlaygroundRuntime()) return;
  await invoke("playground_webview_close", withWindowLabel({ sid }));
}

export async function closeAllPlaygroundWebviews(): Promise<void> {
  if (!isNativePlaygroundRuntime()) return;
  await invoke("playground_webview_close_all", withWindowLabel({}));
}

export async function inspectPlaygroundWebview(
  sid: string,
): Promise<PlaygroundInspectResult> {
  const fallback = { webviewId: playgroundWebviewId(sid) };
  const result = await invokePlayground(
    "playground_webview_inspect",
    { sid },
    fallback,
  );
  if (!isPlaygroundInspectTarget(result.webviewId)) {
    throw new Error("Inspect must target the playground webview.");
  }
  return result;
}

/** Close the WebKit inspector for a playground (e.g. leaving fullscreen). */
export async function closePlaygroundWebviewInspect(
  sid: string,
): Promise<void> {
  if (!isNativePlaygroundRuntime()) return;
  await invoke("playground_webview_close_inspect", withWindowLabel({ sid }));
}

export async function playgroundWebviewGoBack(
  sid: string,
): Promise<PlaygroundNavState> {
  return invokePlayground(
    "playground_webview_go_back",
    { sid },
    {
      sid,
      ...EMPTY_NAV,
    },
  );
}

export async function playgroundWebviewGoForward(
  sid: string,
): Promise<PlaygroundNavState> {
  return invokePlayground(
    "playground_webview_go_forward",
    { sid },
    {
      sid,
      ...EMPTY_NAV,
    },
  );
}

export async function playgroundWebviewReload(sid: string): Promise<void> {
  await invokePlayground("playground_webview_reload", { sid }, undefined);
}

export async function playgroundWebviewNavigate(
  sid: string,
  url: string,
): Promise<PlaygroundNavState> {
  return invokePlayground(
    "playground_webview_navigate",
    { sid, url },
    {
      sid,
      ...EMPTY_NAV,
      currentUrl: url,
    },
  );
}

export async function getPlaygroundWebviewNavState(
  sid: string,
  fallbackUrl = "",
): Promise<PlaygroundNavState> {
  return invokePlayground(
    "playground_webview_nav_state",
    { sid },
    {
      sid,
      ...EMPTY_NAV,
      currentUrl: fallbackUrl,
    },
  );
}

export async function pollPlaygroundWebview(
  sid: string,
  startUrl: string,
): Promise<PlaygroundPollResult> {
  return invokePlayground(
    "playground_webview_poll",
    { sid, startUrl },
    { changed: false },
  );
}

export async function evalPlaygroundWebview(
  sid: string,
  js: string,
): Promise<string> {
  return invokePlayground("playground_webview_eval", { sid, js }, "");
}

export async function playgroundWebviewDomHash(
  sid: string,
  startUrl: string,
): Promise<string> {
  return invokePlayground("playground_webview_dom_hash", { sid, startUrl }, "");
}


/**
 * True when the native playground WKWebView for this sid+window exists.
 * False after app restart (sessions may still be in localStorage), before
 * first Open/show, or after close/dispose. Hide/park keeps the webview → true.
 */
export async function isPlaygroundWebviewOpen(
  sid: string,
  windowLabel = "main",
): Promise<boolean> {
  if (!isNativePlaygroundRuntime()) return false;
  try {
    return await invoke<boolean>("playground_webview_is_open", {
      sid,
      windowLabel: windowLabel.trim() || "main",
    });
  } catch {
    return false;
  }
}

export async function screenshotPlaygroundWebview(
  sid: string,
): Promise<PlaygroundScreenshotPayload> {
  return invokePlayground(
    "playground_webview_screenshot",
    { sid },
    {
      bytes: [],
      mime: "image/png",
      filename: `playground-${sid}.png`,
    },
  );
}

export function subscribePlaygroundWebviewNav(
  onNav: (state: PlaygroundNavState) => void,
): Promise<() => void> {
  if (!isNativePlaygroundRuntime()) {
    return Promise.resolve(() => undefined);
  }
  return listen<PlaygroundNavState>("playground-webview-nav", (event) => {
    onNav(event.payload);
  });
}

export type PlaygroundNewTabRequest = {
  openerSid: string;
  openerLabel: string;
  url: string;
};

/** window.open / target=_blank from a playground WKWebView (Rust MVP B Deny path). */
export function subscribePlaygroundNewTab(
  onRequest: (payload: PlaygroundNewTabRequest) => void,
): Promise<() => void> {
  if (!isNativePlaygroundRuntime()) return Promise.resolve(() => undefined);
  return listen<PlaygroundNewTabRequest>("playground-webview-new-tab", (event) => {
    onRequest(event.payload);
  });
}
