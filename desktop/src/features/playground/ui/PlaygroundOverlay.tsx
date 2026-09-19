import * as React from "react";
import { createPortal } from "react-dom";

import { cn } from "@/shared/lib/cn";

import {
  playgroundConversationHasOpenThread,
  type PlaygroundConversation,
} from "../lib/conversation";
import {
  PLAYGROUND_CHANNEL_THREAD_PANEL_TEST_ID,
  playgroundOverlayPlacement,
  playgroundOverlayShouldPortal,
  readPlaygroundDockThreadEdge,
} from "../lib/dock";
import {
  PLAYGROUND_DOCK_RESIZE_HANDLE_CLASS,
  PLAYGROUND_DOCK_RESIZE_HANDLE_TEST_ID,
  PLAYGROUND_FULLSCREEN_TITLEBAR_GAP_TEST_ID,
  PLAYGROUND_OPAQUE_FILL_STYLE,
  PLAYGROUND_OVERLAY_SURFACE_CLASS,
  playgroundChromeLayoutFlags,
  playgroundFullscreenTitlebarGapClass,
  playgroundOverlayPlacementClass,
  playgroundShowsTitlebarGap,
  playgroundStageLayoutKey,
} from "../lib/overlayLayout";
import {
  currentPopoutPayload,
  openPopoutWindow,
  popoutErrorMessage,
} from "@/features/popout/lib/popoutWindow";
import { toast } from "sonner";
import {
  PLAYGROUND_HULA,
  PLAYGROUND_VERSION,
} from "@/features/playground/lib/types";
import type { PlaygroundSession } from "../lib/sessions";
import { usePlaygroundDockWidth } from "../lib/usePlaygroundDockWidth";
import { PlaygroundChrome } from "./PlaygroundChrome";
import { PlaygroundStage, type PlaygroundChromeMode } from "./PlaygroundStage";

export function PlaygroundOverlay({
  conversation = null,
  lockPlacement,
  session,
}: {
  conversation?: PlaygroundConversation | null;
  lockPlacement?: "window" | "dock";
  session: PlaygroundSession;
}) {
  const overlayRef = React.useRef<HTMLDivElement | null>(null);
  const [mode, setMode] = React.useState<PlaygroundChromeMode>("desktop");
  const [fullscreen, setFullscreen] = React.useState(false);
  const [docked, setDocked] = React.useState(lockPlacement === "dock");
  const [layoutEpoch, setLayoutEpoch] = React.useState(0);

  const getMainWidth = React.useCallback(() => {
    const parent = overlayRef.current?.parentElement;
    if (parent && parent.clientWidth > 0) {
      return parent.clientWidth;
    }
    return typeof window === "undefined" ? 0 : window.innerWidth;
  }, []);

  const getThreadEdge = React.useCallback(() => {
    // Split pop-outs are real panes. The thread is the other flex child,
    // not a channel column to overlay up to.
    if (lockPlacement === "dock") return null;
    if (!playgroundConversationHasOpenThread(conversation)) return null;
    const main = overlayRef.current?.parentElement;
    if (!main) return null;
    const thread = main.querySelector(
      `[data-testid="${PLAYGROUND_CHANNEL_THREAD_PANEL_TEST_ID}"]`,
    );
    return readPlaygroundDockThreadEdge(main, thread);
  }, [conversation, lockPlacement]);

  const { onResetWidth, onResizeStart, prepareDockWidth, widthPx } =
    usePlaygroundDockWidth(getMainWidth, getThreadEdge);

  const bumpStageLayout = React.useCallback(() => {
    setLayoutEpoch((value) => value + 1);
  }, []);

  const setOverlayFullscreen = React.useCallback((next: boolean) => {
    setFullscreen(next);
    setLayoutEpoch((value) => value + 1);
  }, []);

  const toggleDock = React.useCallback(() => {
    if (!docked) {
      prepareDockWidth();
    }
    setDocked((current) => !current);
    setLayoutEpoch((value) => value + 1);
  }, [docked, prepareDockWidth]);

  React.useEffect(() => {
    if (!fullscreen) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      setOverlayFullscreen(false);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [fullscreen, setOverlayFullscreen]);

  const placement = playgroundOverlayPlacement(fullscreen, docked);
  const dockVisible = placement === "dock";
  const chromeLayout = playgroundChromeLayoutFlags(lockPlacement, {
    isOsPopout: currentPopoutPayload() != null,
  });

  async function handleDetach() {
    try {
      await openPopoutWindow({
        kind: "playground",
        title: session.name,
        seed: session.sid,
        playground: {
          hula: PLAYGROUND_HULA,
          v: PLAYGROUND_VERSION,
          name: session.name,
          url: session.url,
          sid: session.sid,
          ...(session.pin ? { pin: session.pin } : {}),
          ...(session.stack ? { stack: session.stack } : {}),
          ...(session.expires != null ? { expires: session.expires } : {}),
        },
      });
    } catch (error) {
      toast.error(popoutErrorMessage(error, "Could not detach playground."));
    }
  }

  // Exiting fullscreen restores dock when they entered from dock
  // (`docked` stays true). Escape / the fullscreen control do not expand.
  const overlay = (
    <div
      className={cn(
        "flex min-h-0 min-w-0 flex-col isolate",
        placement !== "dock" && "overflow-hidden",
        PLAYGROUND_OVERLAY_SURFACE_CLASS,
        playgroundOverlayPlacementClass(placement, lockPlacement === "dock"),
      )}
      data-docked={docked ? "true" : undefined}
      data-fullscreen={fullscreen ? "true" : undefined}
      data-testid="playground-overlay"
      ref={overlayRef}
      style={{
        ...PLAYGROUND_OPAQUE_FILL_STYLE,
        ...(dockVisible ? { width: widthPx } : undefined),
      }}
    >
      {playgroundShowsTitlebarGap(fullscreen) ? (
        <div
          aria-hidden
          className={cn("shrink-0", playgroundFullscreenTitlebarGapClass)}
          data-tauri-drag-region
          data-testid={PLAYGROUND_FULLSCREEN_TITLEBAR_GAP_TEST_ID}
          style={PLAYGROUND_OPAQUE_FILL_STYLE}
        />
      ) : null}
      <PlaygroundChrome
        conversation={conversation}
        docked={docked}
        fullscreen={fullscreen}
        hideDismiss={chromeLayout.hideDismiss}
        hideDispose={chromeLayout.hideDispose}
        hideDock={chromeLayout.hideDock}
        lockPlacement={lockPlacement}
        mode={mode}
        onModeChange={setMode}
        onStageResync={bumpStageLayout}
        onToggleDock={toggleDock}
        onToggleFullscreen={() => setOverlayFullscreen(!fullscreen)}
        session={session}
        onDetach={() => void handleDetach()}
        showDetach={chromeLayout.showDetach}
        showFullscreen={chromeLayout.showFullscreen}
        showInspect={chromeLayout.showInspect}
      />
      <PlaygroundStage
        layoutKey={playgroundStageLayoutKey(fullscreen, layoutEpoch, docked)}
        mode={mode}
        session={session}
      />
      {dockVisible ? (
        <button
          aria-label="Resize playground dock"
          className={PLAYGROUND_DOCK_RESIZE_HANDLE_CLASS}
          data-testid={PLAYGROUND_DOCK_RESIZE_HANDLE_TEST_ID}
          onDoubleClick={onResetWidth}
          onPointerDown={onResizeStart}
          type="button"
        >
          <span className="absolute inset-y-0 left-1/2 w-px -translate-x-1/2 bg-transparent" />
        </button>
      ) : null}
    </div>
  );

  // SidebarInset is `isolate z-0 overflow-hidden` under AppTopChrome `z-45`.
  // A local `fixed inset-0 z-50` cannot paint over that frosted strip (or
  // receive its clicks). Portal to body so fullscreen chrome is the only
  // hit target below the traffic lights. Docked mode stays in the inset so
  // the uncovered right side is real, clickable chat/thread.
  if (
    playgroundOverlayShouldPortal(fullscreen) &&
    typeof document !== "undefined"
  ) {
    return createPortal(overlay, document.body);
  }
  return overlay;
}
