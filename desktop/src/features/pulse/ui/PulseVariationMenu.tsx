import { MoreHorizontal, RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

export function PulseVariationMenu({
  value,
  onChange,
  onRefresh,
  refreshing,
}: {
  value: "separate" | "combined";
  onChange: (value: "separate" | "combined") => void;
  onRefresh: () => void;
  refreshing: boolean;
}) {
  const [host, setHost] = useState<HTMLElement | null>(null);
  useEffect(() => {
    setHost(document.getElementById("app-top-chrome-content"));
  }, []);
  if (!host) return null;
  return createPortal(
    <div className="ml-auto flex items-center">
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
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            size="icon"
            variant="ghost"
            className="h-[28px] w-[28px] shrink-0 rounded-full [&_svg]:size-[16px]"
            aria-label="App variations"
            title="App variations"
          >
            <MoreHorizontal aria-hidden className="h-4 w-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuLabel>App variations</DropdownMenuLabel>
          <DropdownMenuRadioGroup
            value={value}
            onValueChange={(next) => {
              if (next === "separate" || next === "combined") onChange(next);
            }}
          >
            <DropdownMenuRadioItem value="separate">
              Separate feeds
            </DropdownMenuRadioItem>
            <DropdownMenuRadioItem value="combined">
              Combined conversations
            </DropdownMenuRadioItem>
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>,
    host,
  );
}
