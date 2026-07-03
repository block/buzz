import * as React from "react";
import {
  ChevronLeft,
  ChevronRight,
  PanelLeftClose,
  PanelLeftOpen,
} from "lucide-react";

import { isMacPlatform } from "@/shared/lib/platform";
import { useIsFullscreen } from "@/shared/lib/useIsFullscreen";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";
import { topChromeBackdrop } from "@/shared/layout/chromeLayout";
import { useOptionalSidebar } from "@/shared/ui/sidebar";

type AppTopChromeProps = {
  canGoBack: boolean;
  canGoForward: boolean;
  onGoBack: () => void;
  onGoForward: () => void;
  hasWorkspaceRail?: boolean;
};

// Fixed px on purpose (button box + glyph): these controls sit beside the
// native macOS traffic lights, which ignore the app's Cmd +/- text zoom, so
// the row must not grow or shrink with the rem scale. Deliberate exception
// to the rem-first rule.
const TOP_CHROME_ICON_BUTTON_CLASS =
  "h-[28px] w-[28px] rounded-[4px] text-sidebar-foreground/65 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground [&_svg]:size-[16px]";
const HISTORY_ICON_BUTTON_CLASS =
  "h-[28px] w-[24px] rounded-[4px] text-sidebar-foreground/65 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground [&_svg]:size-[16px]";

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
      {sidebar?.open ? <PanelLeftClose /> : <PanelLeftOpen />}
      <span className="sr-only">Toggle Sidebar</span>
    </Button>
  );
}

export function AppTopChrome({
  canGoBack,
  canGoForward,
  onGoBack,
  onGoForward,
  hasWorkspaceRail = false,
}: AppTopChromeProps) {
  const topChromeRef = React.useRef<HTMLDivElement>(null);
  const isFullscreen = useIsFullscreen();
  // On macOS the traffic-light buttons overlay the chrome (see
  // `trafficLightPosition` in `tauri.conf.json`), so the nav row clears their
  // x-position. When the workspace rail is present it already occupies the far
  // left, so the nav row only needs to clear the lights past the rail edge
  // rather than the full offset. In fullscreen those buttons hide.
  //
  // Fixed px on purpose: the native traffic lights do not scale with the app's
  // Cmd +/- text zoom (rem), so rem-based clearance shrinks under them when
  // zoomed out. This is a deliberate exception to the rem-first rule.
  const macChrome = isMacPlatform() && !isFullscreen;
  const navRowPaddingClass = macChrome
    ? hasWorkspaceRail
      ? "pl-[32px]"
      : "pl-[80px]"
    : "pl-3";
  const navRowAlignmentClass = macChrome ? "translate-y-[3px]" : null;

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
    >
      <div className={cn("flex items-center gap-0.5", navRowAlignmentClass)}>
        <TopChromeSidebarTrigger />
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
      </div>
    </div>
  );
}
