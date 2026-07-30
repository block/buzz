import { Bookmark } from "lucide-react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useSavedBookmarks } from "@/features/bookmarks/lib/BookmarksContext";
import { formatFullDateTime } from "@/features/messages/lib/dateFormatters";
import { cn } from "@/shared/lib/cn";
import { UserAvatar } from "@/shared/ui/UserAvatar";

/**
 * "Saved" view — lists the current user's bookmarked messages (newest first)
 * with jump-to-context. Reads the private, encrypted bookmark list surfaced by
 * the app-level `BookmarksProvider`.
 */
export function SavedScreen() {
  const savedEntries = useSavedBookmarks();
  const { goChannel } = useAppNavigation();

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex shrink-0 items-center gap-2 border-b border-border/60 px-4 py-3">
        <Bookmark className="h-4 w-4 text-muted-foreground" />
        <h1 className="text-base font-semibold">Saved</h1>
        {savedEntries.length > 0 ? (
          <span className="text-2xs tabular-nums text-muted-foreground">
            {savedEntries.length}
          </span>
        ) : null}
      </header>

      {savedEntries.length === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-2 px-6 text-center">
          <Bookmark className="h-8 w-8 text-muted-foreground/50" />
          <p className="text-base font-medium">Nothing saved yet</p>
          <p className="max-w-xs text-sm text-muted-foreground">
            Hover any message and tap the bookmark icon to save it for later.
            Your saved messages are private to you.
          </p>
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto py-2">
          {savedEntries.map(({ eventId, entry }) => (
            <button
              className={cn(
                "flex w-full items-start gap-3 px-4 py-2.5 text-left transition-colors",
                "hover:bg-muted/50 focus-visible:bg-muted/50 focus-visible:outline-none",
              )}
              key={eventId}
              onClick={() => {
                void goChannel(entry.channelId, {
                  messageId: eventId,
                  threadRootId: entry.threadRootId ?? null,
                });
              }}
              type="button"
            >
              <UserAvatar
                avatarUrl={null}
                displayName={entry.authorName ?? "Unknown"}
                size="sm"
              />
              <div className="min-w-0 flex-1">
                <div className="flex items-baseline gap-2">
                  <span className="truncate text-base font-medium">
                    {entry.authorName ?? "Unknown"}
                  </span>
                  {entry.createdAt ? (
                    <span className="shrink-0 text-2xs tabular-nums text-muted-foreground">
                      {formatFullDateTime(entry.createdAt)}
                    </span>
                  ) : null}
                </div>
                <p className="line-clamp-2 text-sm text-muted-foreground">
                  {entry.preview?.trim() || "(no preview)"}
                </p>
              </div>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
