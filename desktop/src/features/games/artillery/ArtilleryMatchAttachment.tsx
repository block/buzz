import { Eye, Radio } from "lucide-react";

import type { ArtilleryMatchStartedEvent } from "@/features/games/artillery/durableProtocol";
import { Button } from "@/shared/ui/button";

/** Channel attachment that opens a durable match as a spectator. */
export function ArtilleryMatchAttachment({
  channelId,
  event,
  rootEventId,
}: {
  channelId: string;
  event: ArtilleryMatchStartedEvent;
  rootEventId: string;
}) {
  const watchMatch = () => {
    const search = new URLSearchParams({
      artilleryChannel: channelId,
      artilleryMatch: event.matchId,
      artilleryRoot: rootEventId,
      lab: "artillery",
    });
    window.location.hash = `/?${search.toString()}`;
  };

  return (
    <div
      className="mt-3 flex max-w-xl flex-wrap items-center gap-3 rounded-xl border border-sky-500/25 bg-sky-500/10 px-3 py-2.5"
      data-testid="artillery-match-attachment"
    >
      <div className="flex min-w-0 flex-1 items-center gap-2">
        <span className="grid h-8 w-8 shrink-0 place-items-center rounded-lg bg-sky-500/15 text-sky-600 dark:text-sky-300">
          <Radio className="h-4 w-4" aria-hidden="true" />
        </span>
        <div className="min-w-0">
          <div className="truncate text-sm font-semibold text-foreground">
            {event.agents.red.name} vs {event.agents.blue.name}
          </div>
          <div className="text-xs text-muted-foreground">
            Durable live match · channel-synchronized replay
          </div>
        </div>
      </div>
      <Button
        data-testid="watch-artillery-match"
        onClick={watchMatch}
        size="sm"
        type="button"
      >
        <Eye aria-hidden="true" /> Watch match
      </Button>
    </div>
  );
}
