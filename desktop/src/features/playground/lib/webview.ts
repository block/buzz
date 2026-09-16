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

function currentWindowLabel(): string {
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

export async function showPlaygroundWebview(input: {
  sid: string;
  url: string;
  bounds: PlaygroundWebviewBounds;
  visible?: boolean;
  userAgent?: string;
}): Promise<PlaygroundNavState> {
  return invokePlayground(
    "playground_webview_show",
    {
      sid: input.sid,
      url: input.url,
      bounds: input.bounds,
      visible: input.visible ?? true,
      userAgent: input.userAgent ?? null,
    },
    { sid: input.sid, ...EMPTY_NAV, currentUrl: input.url },
  );
}

export async function hidePlaygroundWebview(sid: string): Promise<void> {
  await invokePlayground("playground_webview_hide", { sid }, undefined);
}

export async function hideAllPlaygroundWebviews(): Promise<void> {
  if (!isNativePlaygroundRuntime()) return;
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
