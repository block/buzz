import * as React from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";

import type { NavigationHistoryEntry } from "@/app/navigation/navigationHistory";
import { isMacPlatform } from "@/shared/lib/platform";
import { useIsFullscreen } from "@/shared/lib/useIsFullscreen";
import { Button } from "@/shared/ui/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/shared/ui/context-menu";
import { DrawerPanelIcon } from "@/shared/ui/DrawerPanelIcon";
import { cn } from "@/shared/lib/cn";
import { topChromeBackdrop } from "@/shared/layout/chromeLayout";
import { useOptionalSidebar } from "@/shared/ui/sidebar";

type AppTopChromeProps = {
  backHistory: NavigationHistoryEntry[];
  canGoBack: boolean;
  canGoForward: boolean;
  forwardHistory: NavigationHistoryEntry[];
  onGoBack: () => void;
  onGoBackTo: (index: number) => void;
  onGoForward: () => void;
  onGoForwardTo: (index: number) => void;
  hasCommunityRail?: boolean;
};

// Fixed px on purpose (button box + glyph): these controls sit beside the
// native macOS traffic lights, which ignore the app's Cmd +/- text zoom, so
// the row must not grow or shrink with the rem scale. Deliberate exception
// to the rem-first rule.
const TOP_CHROME_ICON_BUTTON_CLASS =
  "h-[28px] w-[28px] rounded-[4px] text-sidebar-foreground/65 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground";
const HISTORY_ICON_BUTTON_CLASS =
  "h-[28px] w-[24px] rounded-[4px] text-sidebar-foreground/65 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground [&_svg]:size-[16px]";
const HISTORY_LONG_PRESS_MS = 500;

function preventTopChromeWheel(event: WheelEvent) {
  event.preventDefault();
}

function TopChromeSidebarTrigger() {
  const sidebar = useOptionalSidebar();

  return (
    <Button
      aria-label="Toggle Sidebar"
      className={TOP_CHROME_ICON_BUTTON_CLASS}
      data-sidebar="trigger"
      disabled={!sidebar}
      onClick={() => {
        sidebar?.toggleSidebar();
      }}
      size="icon"
      type="button"
      variant="ghost"
    >
      <DrawerPanelIcon side={sidebar?.open ? "left" : "right"} />
      <span className="sr-only">Toggle Sidebar</span>
    </Button>
  );
}

type HistoryButtonProps = {
  canGo: boolean;
  direction: "back" | "forward";
  entries: NavigationHistoryEntry[];
  onGo: () => void;
  onGoTo: (index: number) => void;
};

function HistoryButton({
  canGo,
  direction,
  entries,
  onGo,
  onGoTo,
}: HistoryButtonProps) {
  const buttonRef = React.useRef<HTMLButtonElement>(null);
  const longPressTimerRef = React.useRef<number | null>(null);
  const longPressTriggeredRef = React.useRef(false);
  const isBack = direction === "back";
  const actionLabel = isBack ? "Go back" : "Go forward";
  const testIdPrefix = isBack ? "global-back" : "global-forward";
  const Icon = isBack ? ChevronLeft : ChevronRight;

  const cancelLongPress = React.useCallback(() => {
    if (longPressTimerRef.current !== null) {
      window.clearTimeout(longPressTimerRef.current);
      longPressTimerRef.current = null;
    }
  }, []);

  React.useEffect(() => cancelLongPress, [cancelLongPress]);

  const handlePointerDown = React.useCallback(
    (event: React.PointerEvent<HTMLButtonElement>) => {
      if (event.button !== 0 || entries.length === 0) {
        return;
      }

      cancelLongPress();
      longPressTriggeredRef.current = false;
      const { clientX, clientY } = event;
      longPressTimerRef.current = window.setTimeout(() => {
        longPressTimerRef.current = null;
        longPressTriggeredRef.current = true;
        buttonRef.current?.dispatchEvent(
          new MouseEvent("contextmenu", {
            bubbles: true,
            button: 2,
            cancelable: true,
            clientX,
            clientY,
            view: window,
          }),
        );
      }, HISTORY_LONG_PRESS_MS);
    },
    [cancelLongPress, entries.length],
  );

  return (
    <ContextMenu
      onOpenChange={(open) => {
        if (!open) {
          longPressTriggeredRef.current = false;
        }
      }}
    >
      <ContextMenuTrigger asChild>
        <Button
          ref={buttonRef}
          aria-label={actionLabel}
          className={HISTORY_ICON_BUTTON_CLASS}
          data-history-count={entries.length}
          data-testid={testIdPrefix}
          disabled={!canGo}
          onClick={(event) => {
            if (longPressTriggeredRef.current) {
              longPressTriggeredRef.current = false;
              event.preventDefault();
              return;
            }

            onGo();
          }}
          onPointerCancel={cancelLongPress}
          onPointerDown={handlePointerDown}
          onPointerLeave={cancelLongPress}
          onPointerUp={cancelLongPress}
          size="icon"
          variant="ghost"
        >
          <Icon />
        </Button>
      </ContextMenuTrigger>
      {entries.length > 0 ? (
        <ContextMenuContent
          className="w-64"
          data-testid={`${testIdPrefix}-history-menu`}
        >
          {entries.map((entry) => (
            <ContextMenuItem
              aria-label={`${actionLabel} to ${entry.label}`}
              data-testid={`${testIdPrefix}-history-item`}
              key={entry.key}
              onSelect={() => onGoTo(entry.index)}
            >
              <span className="min-w-0 truncate">{entry.label}</span>
            </ContextMenuItem>
          ))}
        </ContextMenuContent>
      ) : null}
    </ContextMenu>
  );
}

export function AppTopChrome({
  backHistory,
  canGoBack,
  canGoForward,
  forwardHistory,
  onGoBack,
  onGoBackTo,
  onGoForward,
  onGoForwardTo,
  hasCommunityRail = false,
}: AppTopChromeProps) {
  const topChromeRef = React.useRef<HTMLDivElement>(null);
  const isFullscreen = useIsFullscreen();
  // On macOS the traffic-light buttons overlay the chrome (see
  // `trafficLightPosition` in `tauri.conf.json`), so the nav row clears their
  // x-position. When the community rail is present it already occupies the far
  // left, so the nav row only needs to clear the lights past the rail edge
  // rather than the full offset. In fullscreen those buttons hide.
  //
  // Fixed px on purpose: the native traffic lights do not scale with the app's
  // Cmd +/- text zoom (rem), so rem-based clearance shrinks under them when
  // zoomed out. This is a deliberate exception to the rem-first rule.
  const macChrome = isMacPlatform() && !isFullscreen;
  const navRowPaddingClass = macChrome
    ? hasCommunityRail
      ? "pl-[32px]"
      : "pl-[80px]"
    : "pl-3";
  const navRowAlignmentClass = macChrome ? "translate-y-[3px]" : null;

  React.useLayoutEffect(() => {
    const topChrome = topChromeRef.current;
    const portalTarget = topChrome?.querySelector<HTMLElement>(
      "#app-top-chrome-content",
    );
    if (!topChrome || !portalTarget) return;

    const updateCenterOffset = () => {
      const portalBounds = portalTarget.getBoundingClientRect();
      const portalCenter = portalBounds.left + portalBounds.width / 2;
      topChrome.style.setProperty(
        "--app-top-chrome-center-offset",
        `${window.innerWidth / 2 - portalCenter}px`,
      );
    };

    updateCenterOffset();
    const observer = new ResizeObserver(updateCenterOffset);
    observer.observe(topChrome);
    observer.observe(portalTarget);
    return () => observer.disconnect();
  }, []);

  React.useEffect(() => {
    const topChrome = topChromeRef.current;
    if (!topChrome) {
      return;
    }

    const options = { capture: true, passive: false };
    topChrome.addEventListener("wheel", preventTopChromeWheel, options);
    return () => {
      topChrome.removeEventListener("wheel", preventTopChromeWheel, options);
    };
  }, []);

  return (
    <div
      ref={topChromeRef}
      className={cn(
        "relative z-45 flex shrink-0 cursor-default select-none items-center bg-sidebar pr-3 text-sidebar-foreground",
        topChromeBackdrop.height,
        navRowPaddingClass,
      )}
      data-tauri-drag-region
      data-testid="app-top-chrome"
      style={
        {
          "--app-top-chrome-center-offset": "0px",
        } as React.CSSProperties
      }
    >
      <div className={cn("flex items-center gap-0.5", navRowAlignmentClass)}>
        <TopChromeSidebarTrigger />
        <HistoryButton
          canGo={canGoBack}
          direction="back"
          entries={backHistory}
          onGo={onGoBack}
          onGoTo={onGoBackTo}
        />
        <HistoryButton
          canGo={canGoForward}
          direction="forward"
          entries={forwardHistory}
          onGo={onGoForward}
          onGoTo={onGoForwardTo}
        />
      </div>
      <div
        className={cn("flex min-w-0 flex-1 items-center", navRowAlignmentClass)}
        data-tauri-drag-region
        id="app-top-chrome-content"
      />
    </div>
  );
}
