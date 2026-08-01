import * as React from "react";

import type { DevMentionTickerItem } from "@/features/dev-mode/lib/mentionTicker";
import { DevChannelMembers } from "@/features/dev-mode/ui/DevChannelMembers";
import { DevWorkingChannelName } from "@/features/dev-mode/ui/DevWorkingChannelName";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

export function DevMentionTickerTopBar({
  channel,
  item,
  macChrome,
  onOpen,
  onShowMembers,
  working,
}: {
  channel: Channel | null;
  item: DevMentionTickerItem | null;
  macChrome: boolean;
  onOpen: () => void;
  onShowMembers: () => void;
  working: boolean;
}) {
  React.useEffect(() => {
    if (!item) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (
        (event.metaKey || event.ctrlKey) &&
        event.shiftKey &&
        event.key.toLowerCase() === "m"
      ) {
        event.preventDefault();
        onOpen();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [item, onOpen]);

  if (item) {
    return (
      <button
        className={cn(
          "flex min-w-0 flex-1 cursor-pointer items-center gap-2 overflow-hidden text-left",
          macChrome && "translate-y-[3px]",
        )}
        data-testid="dev-mode-mention-ticker"
        onClick={onOpen}
        title="Jump to mention (⌘⇧M)"
        type="button"
      >
        <span
          className={cn(
            "shrink-0 font-semibold uppercase tracking-wide",
            item.blocked ? "text-destructive" : "text-primary",
          )}
        >
          {item.blocked ? "blocked" : "mention"}
        </span>
        <span className="shrink-0 text-foreground"># {item.channelName}</span>
        <span className="min-w-0 flex-1 truncate text-muted-foreground">
          {item.content}
        </span>
        <kbd className="shrink-0 text-2xs text-muted-foreground/70">⌘⇧M</kbd>
      </button>
    );
  }

  return (
    <>
      <span
        className={cn(
          "pointer-events-none min-w-0 truncate whitespace-nowrap text-foreground",
          macChrome && "translate-y-[3px]",
        )}
        data-testid="dev-mode-topbar-channel"
      >
        {channel ? (
          <>
            #{" "}
            <DevWorkingChannelName name={channel.name} working={working} />
          </>
        ) : null}
      </span>
      {channel ? (
        <span
          className={cn(
            "flex min-w-0 shrink-0 items-baseline",
            macChrome && "translate-y-[3px]",
          )}
        >
          <DevChannelMembers channel={channel} onShowMembers={onShowMembers} />
        </span>
      ) : null}
    </>
  );
}
