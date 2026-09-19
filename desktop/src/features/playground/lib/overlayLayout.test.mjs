import assert from "node:assert/strict";
import test from "node:test";

import {
  PLAYGROUND_CHROME_CLASS,
  PLAYGROUND_FULLSCREEN_OVERLAY_CLASS,
  PLAYGROUND_FULLSCREEN_TITLEBAR_GAP_TEST_ID,
  PLAYGROUND_OPAQUE_FILL_STYLE,
  PLAYGROUND_OVERLAY_SURFACE_CLASS,
  PLAYGROUND_DOCKED_OVERLAY_CLASS,
  PLAYGROUND_SPLIT_PANE_OVERLAY_CLASS,
  PLAYGROUND_DOCK_RESIZE_HANDLE_CLASS,
  PLAYGROUND_RESIZE_HANDLE_CLASS,
  PLAYGROUND_RESIZE_HANDLE_GUTTER_CLASS,
  PLAYGROUND_RESIZE_HANDLE_PX,
  PLAYGROUND_WINDOWED_OVERLAY_CLASS,
  playgroundFullscreenDragRegionIsGapOnly,
  playgroundFullscreenOverlayIsPortaled,
  playgroundFullscreenTitlebarGapClass,
  playgroundOverlayPlacementClass,
  playgroundOverlaySurfaceIsOpaque,
  playgroundChromeLayoutFlags,
  playgroundResizeHandleSitsOutsideHost,
  playgroundShowsTitlebarGap,
  playgroundStageLayoutKey,
} from "./overlayLayout.ts";

test("titlebar gap shows only for fullscreen, not locked pop-outs", () => {
  assert.equal(playgroundShowsTitlebarGap(false), false);
  assert.equal(playgroundShowsTitlebarGap(false, null), false);
  assert.equal(playgroundShowsTitlebarGap(true), true);
  assert.equal(playgroundShowsTitlebarGap(false, "window"), false);
  assert.equal(playgroundShowsTitlebarGap(false, "dock"), false);
  assert.equal(playgroundShowsTitlebarGap(true, "dock"), true);
});

test("fullscreen titlebar gap matches the app chrome strip", () => {
  assert.equal(
    PLAYGROUND_FULLSCREEN_TITLEBAR_GAP_TEST_ID,
    "playground-fullscreen-titlebar-gap",
  );
  assert.match(playgroundFullscreenTitlebarGapClass, /buzz-top-chrome-height/);
  assert.match(playgroundFullscreenTitlebarGapClass, /40px/);
  assert.equal(
    playgroundOverlaySurfaceIsOpaque(playgroundFullscreenTitlebarGapClass),
    true,
  );
});

test("overlay surface is fully opaque", () => {
  assert.equal(PLAYGROUND_OVERLAY_SURFACE_CLASS, "bg-background");
  assert.doesNotMatch(
    PLAYGROUND_FULLSCREEN_OVERLAY_CLASS,
    /backdrop-blur|\/\d+/,
  );
  assert.doesNotMatch(PLAYGROUND_WINDOWED_OVERLAY_CLASS, /backdrop-blur|\/\d+/);
  assert.doesNotMatch(PLAYGROUND_DOCKED_OVERLAY_CLASS, /backdrop-blur|\/\d+/);
  assert.doesNotMatch(PLAYGROUND_DOCKED_OVERLAY_CLASS, /inset-0/);
  assert.match(PLAYGROUND_DOCKED_OVERLAY_CLASS, /left-0/);
  assert.match(PLAYGROUND_DOCKED_OVERLAY_CLASS, /pr-2/);
  assert.match(PLAYGROUND_DOCK_RESIZE_HANDLE_CLASS, /cursor-col-resize/);
  assert.match(PLAYGROUND_DOCK_RESIZE_HANDLE_CLASS, /\bw-2\b/);
  assert.doesNotMatch(PLAYGROUND_DOCK_RESIZE_HANDLE_CLASS, /translate-x/);
  assert.equal(
    playgroundOverlayPlacementClass("dock"),
    PLAYGROUND_DOCKED_OVERLAY_CLASS,
  );
  assert.equal(
    playgroundOverlayPlacementClass("dock", true),
    PLAYGROUND_SPLIT_PANE_OVERLAY_CLASS,
  );
  assert.doesNotMatch(PLAYGROUND_SPLIT_PANE_OVERLAY_CLASS, /absolute/);
  assert.match(PLAYGROUND_SPLIT_PANE_OVERLAY_CLASS, /relative/);
  assert.match(PLAYGROUND_SPLIT_PANE_OVERLAY_CLASS, /shrink-0/);
  assert.match(PLAYGROUND_SPLIT_PANE_OVERLAY_CLASS, /pr-2/);
  assert.equal(
    playgroundOverlayPlacementClass("window"),
    PLAYGROUND_WINDOWED_OVERLAY_CLASS,
  );
  assert.equal(
    playgroundOverlaySurfaceIsOpaque(PLAYGROUND_OVERLAY_SURFACE_CLASS),
    true,
  );
  assert.equal(playgroundOverlaySurfaceIsOpaque(PLAYGROUND_CHROME_CLASS), true);
  assert.match(PLAYGROUND_CHROME_CLASS, /flex-col/);
  assert.match(PLAYGROUND_CHROME_CLASS, /shrink-0/);
  assert.match(PLAYGROUND_CHROME_CLASS, /overflow-visible/);
  assert.doesNotMatch(PLAYGROUND_CHROME_CLASS, /overflow-hidden/);
  assert.doesNotMatch(PLAYGROUND_CHROME_CLASS, /backdrop-blur|\/\d+/);
  assert.equal(
    playgroundOverlaySurfaceIsOpaque("absolute inset-0 z-30 bg-background/95"),
    false,
  );
  assert.equal(
    playgroundOverlaySurfaceIsOpaque("bg-background backdrop-blur"),
    false,
  );
  assert.equal(
    playgroundOverlaySurfaceIsOpaque("bg-background/80 backdrop-blur-md"),
    false,
  );
  assert.match(PLAYGROUND_OPAQUE_FILL_STYLE.backgroundColor, /\/ 1\)$/);
  assert.doesNotMatch(PLAYGROUND_OPAQUE_FILL_STYLE.backgroundColor, /backdrop/);
});

test("fullscreen drag region is only the titlebar gap", () => {
  const gap = {
    hasAttribute: (name) => name === "data-tauri-drag-region",
    contains: () => false,
  };
  const chrome = {
    hasAttribute: () => false,
    querySelector: () => null,
  };
  assert.equal(playgroundFullscreenDragRegionIsGapOnly(gap, chrome), true);
  assert.equal(
    playgroundFullscreenDragRegionIsGapOnly(gap, {
      hasAttribute: (name) => name === "data-tauri-drag-region",
      querySelector: () => null,
    }),
    false,
  );
  assert.equal(
    playgroundFullscreenDragRegionIsGapOnly(
      { ...gap, contains: () => true },
      chrome,
    ),
    false,
  );
});

test("fullscreen overlay is portaled onto document.body", () => {
  const body = {};
  const overlay = { parentElement: body, ownerDocument: { body } };
  assert.equal(playgroundFullscreenOverlayIsPortaled(overlay), true);
  assert.equal(
    playgroundFullscreenOverlayIsPortaled({
      parentElement: {},
      ownerDocument: { body },
    }),
    false,
  );
});

test("resize handles sit outside the webview host", () => {
  assert.equal(PLAYGROUND_RESIZE_HANDLE_PX, 8);
  assert.match(PLAYGROUND_RESIZE_HANDLE_GUTTER_CLASS, /pr-2/);
  assert.match(PLAYGROUND_RESIZE_HANDLE_GUTTER_CLASS, /pb-2/);
  assert.match(PLAYGROUND_RESIZE_HANDLE_CLASS.x, /w-2/);
  assert.match(PLAYGROUND_RESIZE_HANDLE_CLASS.y, /h-2/);
  assert.match(PLAYGROUND_RESIZE_HANDLE_CLASS.xy, /h-2/);
  assert.match(PLAYGROUND_DOCK_RESIZE_HANDLE_CLASS, /\bw-2\b/);
  assert.match(PLAYGROUND_DOCKED_OVERLAY_CLASS, /pr-2/);
  const host = { contains: (node) => node === host };
  const handle = {};
  assert.equal(playgroundResizeHandleSitsOutsideHost(host, handle), true);
  assert.equal(playgroundResizeHandleSitsOutsideHost(host, host), false);
});

test("stage layout key changes when fullscreen or dock toggles", () => {
  assert.equal(playgroundStageLayoutKey(false, 0), "window:0");
  assert.equal(playgroundStageLayoutKey(true, 0), "fullscreen:0");
  assert.equal(playgroundStageLayoutKey(false, 0, true), "dock:0");
  assert.equal(playgroundStageLayoutKey(true, 0, true), "fullscreen:0");
  assert.notEqual(
    playgroundStageLayoutKey(false, 0),
    playgroundStageLayoutKey(true, 0),
  );
  assert.notEqual(
    playgroundStageLayoutKey(false, 0),
    playgroundStageLayoutKey(false, 0, true),
  );
  assert.notEqual(
    playgroundStageLayoutKey(true, 0),
    playgroundStageLayoutKey(true, 1),
  );
});

test("locked chrome hides dispose/dock; Detach in-main; Inspect only when detached", () => {
  assert.deepEqual(playgroundChromeLayoutFlags(), {
    hideDispose: false,
    hideDock: false,
    hideDismiss: false,
    showFullscreen: false,
    showDetach: true,
    showInspect: false,
  });
  assert.deepEqual(playgroundChromeLayoutFlags("window"), {
    hideDispose: true,
    hideDock: true,
    hideDismiss: false,
    showFullscreen: false,
    showDetach: false,
    showInspect: true,
  });
  assert.deepEqual(playgroundChromeLayoutFlags("window", { isOsPopout: true }), {
    hideDispose: true,
    hideDock: true,
    hideDismiss: true,
    showFullscreen: false,
    showDetach: false,
    showInspect: true,
  });
  assert.deepEqual(playgroundChromeLayoutFlags("dock"), {
    hideDispose: true,
    hideDock: true,
    hideDismiss: false,
    showFullscreen: false,
    showDetach: false,
    showInspect: true,
  });
  assert.deepEqual(playgroundChromeLayoutFlags("dock", { isOsPopout: true }), {
    hideDispose: true,
    hideDock: true,
    hideDismiss: true,
    showFullscreen: false,
    showDetach: false,
    showInspect: true,
  });
});
