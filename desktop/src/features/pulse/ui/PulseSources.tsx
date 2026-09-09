import { Globe2, Hash, LockKeyhole, SlidersHorizontal } from "lucide-react";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import type { Channel } from "@/shared/api/types";
import { PULSE_SOURCE_LIMIT } from "@/features/pulse/lib/unifiedFeed";

export function PulseSources({
  channels,
  excludedIds,
  toggleSource,
  includeNotes,
  setIncludeNotes,
}: {
  channels: Channel[];
  excludedIds: ReadonlySet<string>;
  toggleSource: (id: string) => void;
  includeNotes: boolean;
  setIncludeNotes: (value: boolean) => void;
}) {
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="outline" size="sm" className="gap-2 rounded-full">
          <SlidersHorizontal aria-hidden className="h-3.5 w-3.5" />
          Sources
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-80 rounded-2xl p-4">
        <h2 className="text-sm font-semibold">Make this feed yours</h2>
        <p className="mb-4 mt-1 text-xs text-muted-foreground">
          Choose what appears here. This visit only; channel membership stays
          the same.
        </p>
        <label className="flex cursor-pointer items-center gap-3 rounded-lg py-2 text-sm">
          <input
            type="checkbox"
            checked={includeNotes}
            onChange={(e) => setIncludeNotes(e.target.checked)}
            className="accent-primary"
          />
          <Globe2 aria-hidden className="h-4 w-4 text-muted-foreground" />
          Notes from people you follow
        </label>
        <div className="my-3 h-px bg-border/60" />
        <div className="max-h-72 space-y-1 overflow-y-auto">
          {channels.map((channel) => {
            const privateSource =
              channel.channelType === "dm" || channel.visibility === "private";
            const Icon = privateSource ? LockKeyhole : Hash;
            return (
              <label
                key={channel.id}
                className="flex cursor-pointer items-center gap-3 rounded-lg px-1 py-2 text-sm hover:bg-muted/50"
              >
                <input
                  type="checkbox"
                  checked={!excludedIds.has(channel.id)}
                  onChange={() => toggleSource(channel.id)}
                  className="accent-primary"
                />
                <Icon
                  aria-hidden
                  className="h-3.5 w-3.5 shrink-0 text-muted-foreground"
                />
                <span className="truncate">{channel.name}</span>
                {channel.channelType === "dm" && (
                  <span className="ml-auto text-2xs text-muted-foreground">
                    DM
                  </span>
                )}
              </label>
            );
          })}
          {!channels.length && (
            <p className="py-3 text-xs text-muted-foreground">
              Join a channel or start a DM to add a source.
            </p>
          )}
        </div>
        {channels.length > PULSE_SOURCE_LIMIT && (
          <p className="mt-3 text-xs text-muted-foreground">
            This prototype reads your {PULSE_SOURCE_LIMIT} most recently active
            selected channels.
          </p>
        )}
      </PopoverContent>
    </Popover>
  );
}
