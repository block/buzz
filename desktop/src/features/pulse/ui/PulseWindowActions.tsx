import { RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { Button } from "@/shared/ui/button";
export function PulseWindowActions({
  onRefresh,
  refreshing,
}: {
  onRefresh: () => void;
  refreshing: boolean;
}) {
  const [host, setHost] = useState<HTMLElement | null>(null);
  useEffect(() => {
    setHost(document.getElementById("app-top-chrome-content"));
  }, []);
  if (!host) return null;
  return createPortal(
    <div data-pulse-window-actions className="ml-auto flex items-center">
      <Button
        variant="ghost"
        size="icon"
        className="h-[28px] w-[28px] rounded-full [&_svg]:size-[14px]"
        aria-label="Refresh messages"
        title="Refresh messages"
        onClick={onRefresh}
        disabled={refreshing}
      >
        <RefreshCw
          aria-hidden
          className={
            refreshing ? "animate-spin motion-reduce:animate-none" : undefined
          }
        />
      </Button>
    </div>,
    host,
  );
}
