import * as React from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { ChevronLeft, ChevronRight, RefreshCw } from "lucide-react";

import {
  SurfaceTabHomeButton,
  SurfaceTabStrip,
} from "@/app/surfaceTabs/SurfaceTabChrome";
import { isMacPlatform } from "@/shared/lib/platform";
import { useIsFullscreen } from "@/shared/lib/useIsFullscreen";
import { Button } from "@/shared/ui/button";
import { DrawerPanelIcon } from "@/shared/ui/DrawerPanelIcon";
import { cn } from "@/shared/lib/cn";
import { topChromeBackdrop } from "@/shared/layout/chromeLayout";
import { useOptionalSidebar } from "@/shared/ui/sidebar";

type AppTopChromeProps = {
  canGoBack: boolean;
  /** Phone layout only: header back button for full-screen conversation views. */
  onPhoneBack?: () => void;
  /** Phone: name of the screen back lands on, shown beside the arrow. */
  phoneBackLabel?: string;
  canGoForward: boolean;
  onGoBack: () => void;
  onGoForward: () => void;
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

function preventTopChromeWheel(event: WheelEvent) {
  const target = event.target;
  if (target instanceof Element && target.closest("[data-surface-tab-strip]")) {
    return;
  }
  event.preventDefault();
}

function reloadDesktopWindow() {
  // Context-menu Reload: the Tauri webview reloads. Not a data refetch.
  if (!isTauri()) {
    window.location.reload();
    return;
  }
  void invoke("reload_webview");
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

export function AppTopChrome({
  canGoBack,
  canGoForward,
  onGoBack,
  onGoForward,
  hasCommunityRail = false,
  onPhoneBack,
  phoneBackLabel,
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
      <div
        className={cn(
          "flex shrink-0 items-center gap-0.5",
          navRowAlignmentClass,
        )}
      >
        {onPhoneBack ? (
          <button
            aria-label={phoneBackLabel ? `Back to ${phoneBackLabel}` : "Back"}
            className="phone-back-button"
            data-testid="phone-back"
            onClick={onPhoneBack}
            type="button"
          >
            <ChevronLeft aria-hidden="true" className="phone-back-chevron" />
            {phoneBackLabel ? <span className="phone-back-label">{phoneBackLabel}</span> : null}
          </button>
        ) : null}
        <span className="contents" data-phone-hide="">
        <TopChromeSidebarTrigger />
        <Button
          aria-label="Refresh"
          className={TOP_CHROME_ICON_BUTTON_CLASS}
          onClick={reloadDesktopWindow}
          size="icon"
          title="Refresh"
          type="button"
          variant="ghost"
        >
          <RefreshCw />
        </Button>
        <Button
          aria-label="Go back"
          className={HISTORY_ICON_BUTTON_CLASS}
          data-testid="global-back"
          disabled={!canGoBack}
          onClick={onGoBack}
          size="icon"
          variant="ghost"
        >
          <ChevronLeft />
        </Button>
        <Button
          aria-label="Go forward"
          className={HISTORY_ICON_BUTTON_CLASS}
          data-testid="global-forward"
          disabled={!canGoForward}
          onClick={onGoForward}
          size="icon"
          variant="ghost"
        >
          <ChevronRight />
        </Button>
        <SurfaceTabHomeButton className={TOP_CHROME_ICON_BUTTON_CLASS} />
        </span>
      </div>
      <span className="contents" data-phone-hide="">
        <SurfaceTabStrip className={navRowAlignmentClass ?? undefined} />
      </span>
      <div
        className={cn("flex min-w-0 flex-1 items-center", navRowAlignmentClass)}
        data-tauri-drag-region
        id="app-top-chrome-content"
      />
    </div>
  );
}
