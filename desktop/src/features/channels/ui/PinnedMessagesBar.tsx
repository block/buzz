import * as React from "react";
import { Pin, X } from "lucide-react";

import { usePinnedMessagesActions } from "@/features/channels/ui/usePinnedMessagesActions";
import type { TimelineMessage } from "@/features/messages/types";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/shared/ui/popover";

function truncateBody(body: string, max = 120): string {
  const singleLine = body.replace(/\s+/g, " ").trim();
  return singleLine.length > max
    ? `${singleLine.slice(0, max).trimEnd()}…`
    : singleLine;
}

/** Resolves a pinned event id against the currently-loaded message set. A
 *  pin can outlive the local window (e.g. an old message pinned long ago
 *  that hasn't been fetched into `messages` yet) — callers fall back to a
 *  generic "Pinned message" label in that case rather than hiding the pin. */
function usePinnedPreviews(
  pinnedEventIds: readonly string[],
  messages: readonly TimelineMessage[],
) {
  return React.useMemo(() => {
    const byId = new Map(messages.map((message) => [message.id, message]));
    return pinnedEventIds.map((eventId) => ({
      eventId,
      message: byId.get(eventId) ?? null,
    }));
  }, [messages, pinnedEventIds]);
}

function PinnedMessageEntry({
  authorLabel,
  bodyPreview,
  onJumpToMessage,
  onUnpin,
}: {
  authorLabel: string;
  bodyPreview: string;
  onJumpToMessage: () => void;
  onUnpin: () => void;
}) {
  return (
    <div className="flex items-center gap-2 rounded-md px-2 py-1.5 hover:bg-muted/60">
      <button
        className="min-w-0 flex-1 text-left"
        onClick={onJumpToMessage}
        type="button"
      >
        <span className="mr-1.5 font-medium text-foreground">
          {authorLabel}
        </span>
        <span className="text-muted-foreground">{bodyPreview}</span>
      </button>
      <Button
        aria-label="Unpin message"
        className="h-6 w-6 shrink-0 rounded-full p-0"
        onClick={onUnpin}
        size="sm"
        type="button"
        variant="ghost"
      >
        <X className="!h-3.5 !w-3.5" />
      </Button>
    </div>
  );
}

/**
 * Compact pinned-messages area shown at the top of a channel/DM's message
 * timeline. Channel/DM-scoped only (not per reply-thread) — any member can
 * pin or unpin, up to `MAX_PINNED_MESSAGES` (3) at a time.
 *
 * Display rule: exactly one pin renders as a single prominent banner; two or
 * three pins collapse into one compact "N pinned messages" bucket that
 * expands into a small popover list. Never stacks multiple full banners.
 */
export function PinnedMessagesBar({
  channelId,
  messages,
  onJumpToMessage,
}: {
  channelId: string | null;
  messages: readonly TimelineMessage[];
  onJumpToMessage: (messageId: string) => void;
}) {
  const { pinnedEventIds, unpin } = usePinnedMessagesActions(channelId);
  const previews = usePinnedPreviews(pinnedEventIds, messages);
  const [expanded, setExpanded] = React.useState(false);

  if (!channelId || previews.length === 0) {
    return null;
  }

  const renderEntry = (entry: (typeof previews)[number]) => (
    <PinnedMessageEntry
      authorLabel={entry.message?.author ?? "Unknown"}
      bodyPreview={
        entry.message ? truncateBody(entry.message.body) : "Pinned message"
      }
      key={entry.eventId}
      onJumpToMessage={() => onJumpToMessage(entry.eventId)}
      onUnpin={() => unpin(entry.eventId)}
    />
  );

  if (previews.length === 1) {
    return (
      <div
        className="flex items-center gap-2 border-b border-border/60 bg-muted/30 px-4 py-2"
        data-testid="pinned-messages-bar"
      >
        <Pin className="h-4 w-4 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1">{renderEntry(previews[0])}</div>
      </div>
    );
  }

  return (
    <div
      className="border-b border-border/60 bg-muted/30 px-2 py-1"
      data-testid="pinned-messages-bar"
    >
      <Popover onOpenChange={setExpanded} open={expanded}>
        <PopoverTrigger asChild>
          <button
            className={cn(
              "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground",
            )}
            data-testid="pinned-messages-bucket-trigger"
            type="button"
          >
            <Pin className="h-4 w-4 shrink-0" />
            <span>{previews.length} pinned messages</span>
          </button>
        </PopoverTrigger>
        <PopoverContent align="start" className="w-80 p-1.5" side="bottom">
          {previews.map(renderEntry)}
        </PopoverContent>
      </Popover>
    </div>
  );
}
