import { ChevronLeft, ChevronRight } from "lucide-react";

import { TopbarSearch } from "@/features/search/ui/TopbarSearch";
import type { Channel, SearchHit } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { SidebarTrigger, useSidebar } from "@/shared/ui/sidebar";

type AppTopChromeProps = {
  canGoBack: boolean;
  canGoForward: boolean;
  channels: Channel[];
  currentPubkey?: string;
  onGoBack: () => void;
  onGoForward: () => void;
  onOpenChannel: (channelId: string) => void;
  onOpenResult: (hit: SearchHit) => void;
  searchHidden?: boolean;
  searchFocusRequest: number;
};

function GlobalTopDivider() {
  const { state } = useSidebar();

  return (
    <div
      aria-hidden="true"
      className="pointer-events-none fixed top-10 z-40 h-px bg-border/35"
      style={{
        left: state === "expanded" ? "var(--sidebar-width)" : 0,
        right: 0,
      }}
    />
  );
}

function CenterColumnTopbarSearch({
  channels,
  currentPubkey,
  onOpenChannel,
  onOpenResult,
  searchFocusRequest,
}: Pick<
  AppTopChromeProps,
  | "channels"
  | "currentPubkey"
  | "onOpenChannel"
  | "onOpenResult"
  | "searchFocusRequest"
>) {
  const { isResizing, state } = useSidebar();

  return (
    <div
      className="pointer-events-none fixed top-[7px] z-45 flex justify-center px-24 transition-[left] duration-200 ease-linear data-[resizing=true]:transition-none"
      data-testid="topbar-search-column"
      data-resizing={isResizing}
      style={{
        left: state === "expanded" ? "var(--sidebar-width)" : 0,
        right: 0,
      }}
    >
      <TopbarSearch
        channels={channels}
        className="pointer-events-auto w-[220px] max-w-full md:w-[300px] lg:w-[360px] xl:w-[420px] 2xl:w-[480px]"
        currentPubkey={currentPubkey}
        focusRequest={searchFocusRequest}
        onOpenChannel={onOpenChannel}
        onOpenResult={onOpenResult}
      />
    </div>
  );
}

const TOP_CHROME_ICON_BUTTON_CLASS =
  "rounded-[4px] text-muted-foreground/70 hover:bg-border/45 hover:text-foreground";

export function AppTopChrome({
  canGoBack,
  canGoForward,
  channels,
  currentPubkey,
  onGoBack,
  onGoForward,
  onOpenChannel,
  onOpenResult,
  searchHidden = false,
  searchFocusRequest,
}: AppTopChromeProps) {
  return (
    <>
      <div
        aria-hidden="true"
        className="fixed inset-x-0 top-0 z-20 h-10 cursor-default select-none"
        data-tauri-drag-region
      />
      <GlobalTopDivider />
      <div className="fixed left-[80px] top-[9px] z-45 flex items-center gap-0.5">
        <SidebarTrigger className={TOP_CHROME_ICON_BUTTON_CLASS} />
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
      {searchHidden ? null : (
        <CenterColumnTopbarSearch
          channels={channels}
          currentPubkey={currentPubkey}
          onOpenChannel={onOpenChannel}
          onOpenResult={onOpenResult}
          searchFocusRequest={searchFocusRequest}
        />
      )}
    </>
  );
}
