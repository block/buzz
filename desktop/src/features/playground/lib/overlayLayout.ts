import { topChromeBackdrop } from "@/shared/layout/chromeLayout";

import {
  playgroundOverlayPlacement,
  type PlaygroundOverlayPlacement,
} from "./dock";

/** Spacer under the overlay titlebar so traffic lights stay clickable. */
export const PLAYGROUND_FULLSCREEN_TITLEBAR_GAP_TEST_ID =
  "playground-fullscreen-titlebar-gap";

/**
 * Solid overlay fill. `--background` is HSL channels with no alpha, so
 * `bg-background` is 100% opaque. Never use `/95`, `/80`, `/70`, `/55`, or
 * backdrop-blur here — AppTopChrome's frosted `headerBase` must not be
 * borrowed, and the previous surface (channel, Inbox) must not show through.
 */
export const PLAYGROUND_OVERLAY_SURFACE_CLASS = "bg-background";

/** In-app fullscreen covers the viewport, including the AppTopChrome strip. */
export const PLAYGROUND_FULLSCREEN_OVERLAY_CLASS =
  "fixed inset-0 z-[110] pointer-events-auto";

/** Windowed overlay stays inside the channel inset, not the titlebar. */
export const PLAYGROUND_WINDOWED_OVERLAY_CLASS = "absolute inset-0 z-30";

/**
 * Left-anchored split of the main inset. Never `inset-0` — that would stretch
 * over chat even when a width is set. Stays in SidebarInset (not portaled).
 */
export const PLAYGROUND_DOCKED_OVERLAY_CLASS =
  "absolute inset-y-0 left-0 z-30 overflow-visible border-r border-border pr-2";

/**
 * In-flow left pane for split pop-outs. Never `absolute` — the thread is
 * the other flex child and must reflow, not sit underneath.
 */
export const PLAYGROUND_SPLIT_PANE_OVERLAY_CLASS =
  "relative z-30 flex h-full min-h-0 shrink-0 flex-col overflow-visible border-r border-border pr-2";

/**
 * Shared 8px thickness for playground edge handles. Native WKWebView eats
 * clicks, so the hit target must be a real gutter *outside* the host — a
 * 1px border under the webview can hover (CSS) and still miss pointerdown.
 */
export const PLAYGROUND_RESIZE_HANDLE_PX = 8;

/**
 * Right-edge dock drag. `w-2` (8px) sits in the docked overlay's `pr-2`
 * gutter so the handle is outside the native WKWebView, same as responsive.
 */
export const PLAYGROUND_DOCK_RESIZE_HANDLE_CLASS =
  "absolute inset-y-0 right-0 z-40 w-2 cursor-col-resize touch-none pointer-events-auto";

export const PLAYGROUND_DOCK_RESIZE_HANDLE_TEST_ID = "playground-dock-resize";

/**
 * Same 40px strip as `AppTopChrome`. Fixed px via the chrome CSS variable so
 * Cmd +/- zoom cannot pull Dispose into the macOS traffic-light hit region.
 * Painted opaque so fullscreen does not leak titlebar vibrancy. This gap is
 * the only `data-tauri-drag-region` — never put drag on the chrome row.
 */
export const playgroundFullscreenTitlebarGapClass = `${topChromeBackdrop.height} ${PLAYGROUND_OVERLAY_SURFACE_CLASS}`;

/**
 * 40px overlay gap so traffic lights stay clickable. Only fullscreen — the
 * portaled overlay leaves SidebarInset. Pop-outs use a window-level strip in
 * AppShellChannelSurface instead; stacking both would give the left pane 80px.
 */
export function playgroundShowsTitlebarGap(fullscreen: boolean): boolean {
  return fullscreen;
}

export const PLAYGROUND_CHROME_CLASS = `relative z-20 flex shrink-0 flex-col gap-1 overflow-visible border-b border-border pointer-events-auto px-2 py-1 ${PLAYGROUND_OVERLAY_SURFACE_CLASS}`;

/**
 * Force alpha 1 so macOS titlebar vibrancy cannot blend through `--background`
 * when the WKWebView canvas is transparent (glass).
 */
export const PLAYGROUND_OPAQUE_FILL_STYLE = {
  backgroundColor: "hsl(var(--background) / 1)",
} as const;

/** 8px gutter so edge handles sit outside the native WKWebView host. */
export const PLAYGROUND_RESIZE_HANDLE_GUTTER_CLASS = "pr-2 pb-2";

export const PLAYGROUND_RESIZE_HANDLE_CLASS = {
  x: "absolute bottom-2 right-0 top-0 z-20 w-2 cursor-ew-resize touch-none rounded-full bg-border",
  y: "absolute bottom-0 left-0 right-2 z-20 h-2 cursor-ns-resize touch-none rounded-full bg-border",
  xy: "absolute bottom-0 right-0 z-20 h-2 w-2 cursor-nwse-resize touch-none rounded-sm bg-border",
} as const;

export function playgroundOverlaySurfaceIsOpaque(className: string): boolean {
  const tokens = className.trim().split(/\s+/);
  return (
    tokens.includes(PLAYGROUND_OVERLAY_SURFACE_CLASS) &&
    !tokens.some(
      (token) => token.includes("backdrop-blur") || /\/\d+$/.test(token),
    )
  );
}

/** Dispose / nav / URL / Inspect must not sit on a Tauri drag region. */
export function playgroundFullscreenDragRegionIsGapOnly(
  gap: Element,
  chrome: Element,
): boolean {
  return (
    gap.hasAttribute("data-tauri-drag-region") &&
    !chrome.hasAttribute("data-tauri-drag-region") &&
    !gap.contains(chrome) &&
    chrome.querySelector("[data-tauri-drag-region]") == null
  );
}

/** Fullscreen must portal to `document.body` so AppTopChrome cannot steal hits. */
export function playgroundFullscreenOverlayIsPortaled(
  overlay: Element,
): boolean {
  return overlay.parentElement === overlay.ownerDocument?.body;
}

export function playgroundResizeHandleSitsOutsideHost(
  host: { contains: (node: Node) => boolean },
  handle: Node,
): boolean {
  return !host.contains(handle);
}

export function playgroundOverlayPlacementClass(
  placement: PlaygroundOverlayPlacement,
  inFlowSplit = false,
): string {
  switch (placement) {
    case "fullscreen":
      return PLAYGROUND_FULLSCREEN_OVERLAY_CLASS;
    case "dock":
      return inFlowSplit
        ? PLAYGROUND_SPLIT_PANE_OVERLAY_CLASS
        : PLAYGROUND_DOCKED_OVERLAY_CLASS;
    default:
      return PLAYGROUND_WINDOWED_OVERLAY_CLASS;
  }
}

export function playgroundStageLayoutKey(
  fullscreen: boolean,
  epoch: number,
  docked = false,
): string {
  return `${playgroundOverlayPlacement(fullscreen, docked)}:${epoch}`;
}

export type PlaygroundChromeLayoutFlags = {
  hideDispose: boolean;
  hideDock: boolean;
  hideDismiss: boolean;
  /** @deprecated Prefer Detach into an OS window; kept false. */
  showFullscreen: boolean;
  /** Detach into a playground OS/embedded window (in-main overlay only). */
  showDetach: boolean;
  /** Inspect only once the playground browser is in its own window. */
  showInspect: boolean;
};

/**
 * Locked placement (split/window) hides dispose and dock.
 * OS pop-outs also hide dismiss (close the OS window instead). In-main embeds
 * keep dismiss so chrome X can park React state and the native WKWebView.
 * Fullscreen is replaced by Detach; Inspect is only available when detached.
 */
export function playgroundChromeLayoutFlags(
  lockPlacement?: "window" | "dock",
  options?: { isOsPopout?: boolean },
): PlaygroundChromeLayoutFlags {
  const isOsPopout = options?.isOsPopout === true;
  const detached = isOsPopout || lockPlacement === "window";
  if (lockPlacement == null) {
    return {
      hideDispose: false,
      hideDock: false,
      hideDismiss: false,
      showFullscreen: false,
      showDetach: !isOsPopout,
      showInspect: detached,
    };
  }
  return {
    hideDispose: true,
    hideDock: true,
    hideDismiss: isOsPopout,
    showFullscreen: false,
    showDetach: false,
    showInspect: detached || lockPlacement === "dock",
  };
}
