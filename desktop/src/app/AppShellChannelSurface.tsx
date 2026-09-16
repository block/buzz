import type * as React from "react";
import { useSyncExternalStore } from "react";
import * as BuzzTheme from "@/app/BuzzThemeSurfaces";
import { HuddleRoomHeader, HuddleStartingView } from "@/features/huddle";
import {
  PLAYGROUND_OPAQUE_FILL_STYLE,
  playgroundFullscreenTitlebarGapClass,
} from "@/features/playground/lib/overlayLayout";
import { PlaygroundHost } from "@/features/playground/ui/PlaygroundHost";
import {
  getActiveEmbeddedWindow,
  getEmbeddedWindowsStore,
  subscribeEmbeddedWindows,
} from "@/features/popout/lib/embeddedWindows";
import { PopoutLayoutProvider } from "@/features/popout/lib/popoutLayout";
import {
  getPopoutSettings,
  isEmbedInMainEnabled,
  subscribePopoutSettings,
} from "@/features/popout/lib/popoutSettings";
import { currentPopoutPayload } from "@/features/popout/lib/popoutWindow";
import { MainInsetProvider } from "@/shared/layout/MainInsetContext";
import { chromeCssVarDefaults } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
import { SidebarInset, useSidebar } from "@/shared/ui/sidebar";

type AppShellChannelSurfaceProps = {
  children: React.ReactNode;
  hasCommunityRail: boolean;
  isHuddleRoom: boolean;
  isHuddleRoomStarting: boolean;
  mainInsetRef: React.RefObject<HTMLElement | null>;
  terminal?: React.ReactNode;
};

export function AppShellChannelSurface({
  children,
  hasCommunityRail,
  isHuddleRoom,
  isHuddleRoomStarting,
  mainInsetRef,
  terminal,
}: AppShellChannelSurfaceProps) {
  const { isMobile, openMobile, state: sidebarState } = useSidebar();
  useSyncExternalStore(
    subscribePopoutSettings,
    getPopoutSettings,
    getPopoutSettings,
  );
  useSyncExternalStore(
    subscribeEmbeddedWindows,
    getEmbeddedWindowsStore,
    getEmbeddedWindowsStore,
  );
  const osPopout = currentPopoutPayload();
  const embed =
    osPopout || !isEmbedInMainEnabled() ? null : getActiveEmbeddedWindow();
  const payload = osPopout ?? embed?.payload ?? null;
  const isOsPopout = osPopout != null;
  const isSplit = payload?.kind === "split";
  const contentUnframed =
    isHuddleRoom ||
    isOsPopout ||
    (embed != null && embed.payload.kind !== "playground");
  const hasCollapsedSidebarGutter =
    !isHuddleRoom &&
    !hasCommunityRail &&
    (isMobile ? !openMobile : sidebarState === "collapsed");

  const panes = (
    <PopoutLayoutProvider payload={isOsPopout ? null : payload}>
      {isHuddleRoom && !isHuddleRoomStarting ? <HuddleRoomHeader /> : null}
      {isSplit ? <PlaygroundHost /> : null}
      <BuzzTheme.ContentSurface
        className={isSplit ? "min-w-0" : undefined}
        terminal={terminal}
        unframed={contentUnframed}
      >
        {isHuddleRoomStarting ? <HuddleStartingView /> : children}
      </BuzzTheme.ContentSurface>
      {isSplit ? null : <PlaygroundHost />}
    </PopoutLayoutProvider>
  );

  const splitHost = isOsPopout || (embed != null && isSplit);

  return (
    <MainInsetProvider mainInsetRef={mainInsetRef}>
      <SidebarInset
        ref={mainInsetRef}
        className={cn(
          "relative isolate z-0 min-h-0 min-w-0 overflow-hidden",
          splitHost && "flex flex-col",
          isHuddleRoom ? "bg-background" : "bg-sidebar",
          hasCollapsedSidebarGutter && "pl-2",
        )}
        data-buzz-content-surface={isHuddleRoom ? true : undefined}
        data-buzz-content-unframed={contentUnframed ? true : undefined}
        data-buzz-glass-inset
        data-buzz-shadow-viewport
        style={chromeCssVarDefaults as React.CSSProperties}
      >
        {hasCollapsedSidebarGutter ? (
          <div
            className="absolute inset-y-0 left-0 w-2 bg-sidebar"
            data-collapsed-content-gutter
          />
        ) : null}
        {isOsPopout ? (
          <>
            <div
              aria-hidden
              className={cn("shrink-0", playgroundFullscreenTitlebarGapClass)}
              data-tauri-drag-region
              data-testid="popout-titlebar-gap"
              style={PLAYGROUND_OPAQUE_FILL_STYLE}
            />
            <div
              className={cn(
                "relative flex min-h-0 min-w-0 flex-1",
                isSplit && "flex-row",
              )}
            >
              {panes}
            </div>
          </>
        ) : embed != null && isSplit ? (
          <div className="relative flex min-h-0 min-w-0 flex-1 flex-row">
            {panes}
          </div>
        ) : (
          panes
        )}
      </SidebarInset>
    </MainInsetProvider>
  );
}
