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
};

const TOP_CHROME_ICON_BUTTON_CLASS =
  "h-7 w-7 rounded-[4px] text-sidebar-foreground/65 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground [&_svg]:size-4";

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
}: AppTopChromeProps) {
  const topChromeRef = React.useRef<HTMLDivElement>(null);
  const isFullscreen = useIsFullscreen();
  // On macOS the traffic-light buttons overlay the chrome (see
  // `trafficLightPosition` in `tauri.conf.json`), so the nav row clears their
  // x-position and shifts to align the nav icon centers with the native dot
  // centers. In fullscreen those buttons hide, so use the standard alignment.
  const navRowPaddingClass =
    isMacPlatform() && !isFullscreen ? "pl-20" : "pl-3";
  const navRowAlignmentClass =
    isMacPlatform() && !isFullscreen ? "translate-y-[3px]" : null;

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
          className={TOP_CHROME_ICON_BUTTON_CLASS}
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
          className={TOP_CHROME_ICON_BUTTON_CLASS}
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
