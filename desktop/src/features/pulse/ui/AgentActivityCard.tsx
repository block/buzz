import { Bot, ChevronDown, ChevronRight } from "lucide-react";
import * as React from "react";

import type { AgentNoteGroup } from "@/features/pulse/lib/groupAgentNotes";
import type { UserProfileSummary } from "@/shared/api/types";
import { Markdown } from "@/shared/ui/markdown";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type AgentActivityCardProps = {
  group: AgentNoteGroup;
  profile?: UserProfileSummary | null;
  agentStatus?: "online" | "away" | "offline";
};

function formatRelativeTime(unixSeconds: number): string {
  const now = Date.now() / 1_000;
  const diff = now - unixSeconds;

  if (diff < 60) return "just now";
  if (diff < 3_600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86_400) return `${Math.floor(diff / 3_600)}h ago`;
  if (diff < 604_800) return `${Math.floor(diff / 86_400)}d ago`;

  return new Date(unixSeconds * 1_000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

function StatusDot({ status }: { status: "online" | "away" | "offline" }) {
  const color =
    status === "online"
      ? "bg-emerald-500"
      : status === "away"
        ? "bg-amber-500"
        : "bg-zinc-400";
  return <span className={`inline-block h-2 w-2 rounded-full ${color}`} />;
}

export function AgentActivityCard({
  group,
  profile,
  agentStatus,
}: AgentActivityCardProps) {
  const [expanded, setExpanded] = React.useState(false);
  const displayName = profile?.displayName ?? `${group.pubkey.slice(0, 8)}...`;
  const avatarUrl = profile?.avatarUrl ?? null;
  const isSingleNote = group.notes.length === 1;

  // Show the latest note content as the summary.
  const summaryNote = group.notes[0];

  return (
    <div className="rounded-2xl px-1 py-4 sm:px-2">
      {/* Header */}
      <div className="flex items-center gap-3">
        <div className="relative shrink-0 pt-1">
          <UserAvatar avatarUrl={avatarUrl} displayName={displayName} />
          <Bot className="absolute -bottom-0.5 -right-0.5 h-3.5 w-3.5 rounded-full bg-background p-0.5 text-muted-foreground" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-semibold leading-none">
              {displayName}
            </span>
            {agentStatus ? <StatusDot status={agentStatus} /> : null}
            <span className="shrink-0 text-[11px] text-muted-foreground">
              {formatRelativeTime(group.latestAt)}
            </span>
          </div>
        </div>
        {!isSingleNote ? (
          <button
            className="flex h-6 items-center gap-1 rounded-full px-2 text-xs text-muted-foreground hover:bg-muted/50 hover:text-foreground"
            onClick={() => setExpanded(!expanded)}
            type="button"
          >
            {expanded ? (
              <ChevronDown className="h-3.5 w-3.5" />
            ) : (
              <ChevronRight className="h-3.5 w-3.5" />
            )}
            {group.notes.length} updates
          </button>
        ) : null}
      </div>

      {/* Content */}
      {isSingleNote || !expanded ? (
        <div className="mt-1.5 ml-[44px] text-sm leading-relaxed text-foreground">
          <Markdown content={summaryNote.content} />
        </div>
      ) : (
        <div className="mt-2 ml-[44px] space-y-2">
          {group.notes.map((note, idx) => (
            <div
              className="flex gap-2 rounded-xl border border-border/50 bg-muted/20 px-3 py-2"
              key={note.id}
            >
              <span className="mt-0.5 shrink-0 text-xs font-medium text-muted-foreground">
                {idx + 1}.
              </span>
              <div className="min-w-0 flex-1 text-sm">
                <Markdown content={note.content} />
                <p className="mt-1 text-[10px] text-muted-foreground">
                  {formatRelativeTime(note.createdAt)}
                </p>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
