import { Activity, Bot } from "lucide-react";

import type { UserNote } from "@/shared/api/socialTypes";
import type { UserProfileSummary } from "@/shared/api/types";
import { Markdown } from "@/shared/ui/markdown";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type RecentNotesSectionProps = {
  notes: UserNote[];
  profiles: Record<string, UserProfileSummary>;
  agentPubkeys: ReadonlySet<string>;
  onOpenPulse: () => void;
};

function formatRelativeTime(unixSeconds: number): string {
  const now = Date.now() / 1_000;
  const diff = now - unixSeconds;

  if (diff < 60) return "just now";
  if (diff < 3_600) return `${Math.floor(diff / 60)}m`;
  if (diff < 86_400) return `${Math.floor(diff / 3_600)}h`;
  return `${Math.floor(diff / 86_400)}d`;
}

export function RecentNotesSection({
  notes,
  profiles,
  agentPubkeys,
  onOpenPulse,
}: RecentNotesSectionProps) {
  if (notes.length === 0) return null;

  return (
    <div>
      <div className="mb-2 flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Activity className="h-4 w-4 text-muted-foreground" />
          <h3 className="text-sm font-semibold text-foreground">
            Recent Notes
          </h3>
        </div>
        <button
          className="text-xs text-primary hover:underline"
          onClick={onOpenPulse}
          type="button"
        >
          View all in Pulse
        </button>
      </div>

      <div className="space-y-0 overflow-hidden rounded-md border border-border/60">
        {notes.slice(0, 5).map((note) => {
          const profile = profiles[note.pubkey.toLowerCase()];
          const displayName =
            profile?.displayName ?? `${note.pubkey.slice(0, 8)}...`;
          const isAgent = agentPubkeys.has(note.pubkey);

          return (
            <div
              className="flex items-start gap-2.5 border-b border-border/40 px-3 py-2.5 last:border-b-0"
              key={note.id}
            >
              <div className="relative shrink-0 pt-0.5">
                <UserAvatar
                  avatarUrl={profile?.avatarUrl ?? null}
                  displayName={displayName}
                  size="sm"
                />
                {isAgent ? (
                  <Bot className="absolute -bottom-0.5 -right-0.5 h-4 w-4 rounded-full bg-background p-0.5 text-muted-foreground" />
                ) : null}
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-1.5">
                  <span className="truncate text-xs font-semibold">
                    {displayName}
                  </span>
                  {isAgent ? (
                    <span className="inline-flex h-3.5 items-center rounded bg-muted px-1 text-2xs font-medium text-muted-foreground">
                      bot
                    </span>
                  ) : null}
                  <span className="shrink-0 text-2xs text-muted-foreground">
                    {formatRelativeTime(note.createdAt)}
                  </span>
                </div>
                <div className="mt-0.5 line-clamp-2 text-xs text-muted-foreground">
                  <Markdown content={note.content} />
                </div>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
