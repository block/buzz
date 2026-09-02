import { LogIn } from "lucide-react";

import { ChannelGlyph } from "@/features/channels/ui/ChannelGlyph";
import type { Channel } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";

export function JoinChannelBanner({
  channel,
  isJoining,
  onJoin,
}: {
  channel: Channel;
  isJoining: boolean;
  onJoin?: () => Promise<void>;
}) {
  return (
    <div
      className="flex items-center gap-3 border-t border-border/80 bg-card/50 px-5 py-3"
      data-testid="join-banner"
    >
      <div className="flex min-w-0 flex-1 items-center gap-2 text-sm text-muted-foreground">
        <ChannelGlyph channel={channel} className="h-4 w-4 shrink-0" />
        <span className="truncate">
          Viewing{" "}
          <span className="font-medium text-foreground">#{channel.name}</span>
        </span>
      </div>
      <Button
        disabled={isJoining}
        onClick={() => void onJoin?.()}
        size="sm"
        variant="default"
      >
        <LogIn className="mr-1.5 h-4 w-4" />
        {isJoining ? "Joining..." : "Join to participate"}
      </Button>
    </div>
  );
}
