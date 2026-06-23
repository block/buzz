import {
  ChevronLeft,
  ChevronRight,
  PanelLeftClose,
  PanelLeftOpen,
} from "lucide-react";
import * as React from "react";

import { isMacPlatform } from "@/shared/lib/platform";
import { useIsFullscreen } from "@/shared/lib/useIsFullscreen";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";
import { useOptionalSidebar } from "@/shared/ui/sidebar";

type AppTopChromeProps = {
  canGoBack: boolean;
  canGoForward: boolean;
  onGoBack: () => void;
  onGoForward: () => void;
};

const TOP_CHROME_ICON_BUTTON_CLASS =
  "h-7 w-7 rounded-[4px] text-muted-foreground/70 hover:bg-border/45 hover:text-foreground [&_svg]:size-4";
const TOP_CHROME_WHEEL_GUARD_HEIGHT = 40;
const TOP_CHROME_RIGHT_PANEL_SAFE_AREA_PX = 420;

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
  const isFullscreen = useIsFullscreen();
  // On macOS the traffic-light buttons overlay the chrome at x≈12 (see
  // `trafficLightPosition` in `tauri.conf.json`), so the nav row sits at
  // `left-[80px]` to clear them. In fullscreen those buttons hide, so shift
  // the row back to the standard left inset.
  const navRowLeftClass =
    isMacPlatform() && isFullscreen ? "left-[12px]" : "left-[80px]";

  React.useEffect(() => {
    const handleWheel = (event: WheelEvent) => {
      if (event.clientY <= TOP_CHROME_WHEEL_GUARD_HEIGHT) {
        event.preventDefault();
      }
    };

    document.addEventListener("wheel", handleWheel, {
      capture: true,
      passive: false,
    });
    return () => {
      document.removeEventListener("wheel", handleWheel, { capture: true });
    };
  }, []);

  return (
    <>
      <div
        aria-hidden="true"
        className="fixed left-0 top-0 z-20 h-10 cursor-default select-none"
        data-tauri-drag-region
        style={{ right: TOP_CHROME_RIGHT_PANEL_SAFE_AREA_PX }}
      />
      <div
        className={cn(
          "fixed top-[6px] z-45 flex items-center gap-0.5",
          navRowLeftClass,
        )}
      >
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
    </>
  );
}
