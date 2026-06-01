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
  searchFocusRequest: number;
};

function GlobalTopDivider() {
  const { state } = useSidebar();

  return (
    <div
      aria-hidden="true"
      className="pointer-events-none fixed right-0 top-10 z-40 h-px bg-border/35"
      style={{ left: state === "expanded" ? "var(--sidebar-width)" : 0 }}
    />
  );
}

export function AppTopChrome({
  canGoBack,
  canGoForward,
  channels,
  currentPubkey,
  onGoBack,
  onGoForward,
  onOpenChannel,
  onOpenResult,
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
      <div className="fixed left-[80px] top-[9px] z-[45] flex items-center gap-0.5">
        <SidebarTrigger className="h-[22px] w-[22px] text-muted-foreground/70 hover:bg-muted/60 hover:text-foreground" />
        <Button
          aria-label="Go back"
          className="h-[22px] w-[22px] text-muted-foreground/70 hover:bg-muted/60 hover:text-foreground"
          data-testid="global-back"
          disabled={!canGoBack}
          onClick={onGoBack}
          size="icon"
          variant="ghost"
        >
          <ChevronLeft className="h-3 w-3" />
        </Button>
        <Button
          aria-label="Go forward"
          className="h-[22px] w-[22px] text-muted-foreground/70 hover:bg-muted/60 hover:text-foreground"
          data-testid="global-forward"
          disabled={!canGoForward}
          onClick={onGoForward}
          size="icon"
          variant="ghost"
        >
          <ChevronRight className="h-3 w-3" />
        </Button>
      </div>
      <TopbarSearch
        channels={channels}
        className="fixed left-1/2 top-[7px] z-[45] block w-[220px] max-w-[calc(100vw-11rem)] -translate-x-1/2 md:w-[300px] md:max-w-[34vw] lg:w-[360px] lg:max-w-[38vw] xl:w-[420px] xl:max-w-[42vw] 2xl:w-[480px] 2xl:max-w-[44vw]"
        currentPubkey={currentPubkey}
        focusRequest={searchFocusRequest}
        onOpenChannel={onOpenChannel}
        onOpenResult={onOpenResult}
      />
    </>
  );
}
