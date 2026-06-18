import { Check, type LucideIcon } from "lucide-react";

import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import type { FeedItem } from "@/shared/api/types";
import {
  KIND_APPROVAL_REQUEST,
  KIND_FORUM_COMMENT,
  KIND_FORUM_POST,
  KIND_JOB_ACCEPTED,
  KIND_JOB_CANCEL,
  KIND_JOB_ERROR,
  KIND_JOB_PROGRESS,
  KIND_JOB_REQUEST,
  KIND_JOB_RESULT,
  KIND_REMINDER,
} from "@/shared/constants/kinds";
import { resolveMentionNames } from "@/shared/lib/resolveMentionNames";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";
import { UserAvatar } from "@/shared/ui/UserAvatar";

const relativeTimeFormatter = new Intl.RelativeTimeFormat("en-US", {
  numeric: "auto",
});

function formatRelativeTime(unixSeconds: number) {
  const diff = unixSeconds - Math.floor(Date.now() / 1_000);
  const absoluteDiff = Math.abs(diff);

  if (absoluteDiff < 60) {
    return relativeTimeFormatter.format(diff, "second");
  }

  if (absoluteDiff < 60 * 60) {
    return relativeTimeFormatter.format(Math.round(diff / 60), "minute");
  }

  if (absoluteDiff < 60 * 60 * 24) {
    return relativeTimeFormatter.format(Math.round(diff / (60 * 60)), "hour");
  }

  if (absoluteDiff < 60 * 60 * 24 * 7) {
    return relativeTimeFormatter.format(
      Math.round(diff / (60 * 60 * 24)),
      "day",
    );
  }

  return new Intl.DateTimeFormat("en-US", {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(new Date(unixSeconds * 1_000));
}

function feedHeadline(item: FeedItem) {
  switch (item.kind) {
    case KIND_REMINDER:
      return "Reminder";
    case KIND_JOB_REQUEST:
      return "Job requested";
    case KIND_JOB_ACCEPTED:
      return "Job accepted";
    case KIND_JOB_PROGRESS:
      return "Progress update";
    case KIND_JOB_RESULT:
      return "Job result";
    case KIND_JOB_CANCEL:
      return "Job cancelled";
    case KIND_JOB_ERROR:
      return "Job failed";
    case KIND_FORUM_POST:
      return "Forum post";
    case KIND_FORUM_COMMENT:
      return "Forum reply";
    case KIND_APPROVAL_REQUEST:
      return "Approval requested";
    default:
      if (item.category === "mention") {
        return "Mention";
      }

      if (item.category === "agent_activity") {
        return "Agent update";
      }

      return "Channel update";
  }
}

function feedContent(item: FeedItem) {
  const content = item.content.trim();
  if (content.length > 0) {
    return content;
  }

  if (item.kind === KIND_APPROVAL_REQUEST) {
    return "A workflow is waiting for approval.";
  }

  if (item.kind === KIND_REMINDER) {
    return "A reminder is waiting for you.";
  }

  return "No additional details were attached to this event.";
}

type FeedSectionProps = {
  title: string;
  emptyTitle: string;
  emptyDescription: string;
  icon: LucideIcon;
  items: FeedItem[];
  currentPubkey?: string;
  profiles?: UserProfileLookup;
  availableChannelIds: ReadonlySet<string>;
  doneSet: ReadonlySet<string>;
  showDoneAction: boolean;
  onOpenItem: (item: FeedItem) => void;
  onMarkDone: (id: string) => void;
  onUndoDone: (id: string) => void;
};

export function FeedSection({
  title,
  emptyTitle,
  emptyDescription,
  icon: Icon,
  items,
  currentPubkey,
  profiles,
  availableChannelIds,
  doneSet,
  showDoneAction,
  onOpenItem,
  onMarkDone,
  onUndoDone,
}: FeedSectionProps) {
  return (
    <section>
      <div className="flex items-center gap-2 pb-2">
        <Icon className="h-4 w-4 text-muted-foreground" />
        <h2 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          {title}
        </h2>
        <span className="text-xs text-muted-foreground/70">{items.length}</span>
      </div>

      {items.length === 0 ? (
        <div className="rounded-md border border-dashed border-border/60 px-4 py-5 text-center">
          <p className="text-sm font-medium text-muted-foreground">
            {emptyTitle}
          </p>
          <p className="mt-1 text-xs text-muted-foreground/70">
            {emptyDescription}
          </p>
        </div>
      ) : (
        <div className="divide-y divide-border/60 rounded-md border border-border/60">
          {items.map((item) => {
            const channelId = item.channelId;
            const canOpenChannel =
              channelId !== null && availableChannelIds.has(channelId);
            const isDone = doneSet.has(item.id);
            const mentionNames = resolveMentionNames(item.tags, profiles);

            return (
              <div
                className={`group relative px-3 py-2.5 transition-colors hover:bg-muted/40 ${isDone ? "opacity-50" : ""} ${canOpenChannel ? "cursor-pointer" : ""}`}
                key={item.id}
              >
                {canOpenChannel ? (
                  <button
                    aria-label={`Open ${item.channelName || "channel"}`}
                    className="absolute inset-0"
                    data-testid={`home-feed-open-${item.id}`}
                    onClick={() => {
                      onOpenItem(item);
                    }}
                    type="button"
                  />
                ) : null}

                <div className="pointer-events-none relative flex min-w-0 items-center gap-2">
                  <span
                    className={`text-sm font-medium ${isDone ? "line-through text-muted-foreground" : ""}`}
                  >
                    {feedHeadline(item)}
                  </span>
                  <span className="inline-flex items-center gap-1 text-2xs text-muted-foreground">
                    <UserAvatar
                      avatarUrl={
                        profiles?.[item.pubkey.toLowerCase()]?.avatarUrl ?? null
                      }
                      displayName={resolveUserLabel({
                        pubkey: item.pubkey,
                        currentPubkey,
                        profiles,
                        preferResolvedSelfLabel: true,
                      })}
                      size="xs"
                    />
                    {resolveUserLabel({
                      pubkey: item.pubkey,
                      currentPubkey,
                      profiles,
                      preferResolvedSelfLabel: true,
                    })}
                  </span>
                  {item.channelName ? (
                    <span className="text-2xs text-primary/80">
                      #{item.channelName}
                    </span>
                  ) : null}
                  <span className="ml-auto shrink-0 text-2xs text-muted-foreground/60">
                    {formatRelativeTime(item.createdAt)}
                  </span>
                </div>

                <div className="pointer-events-none relative mt-0.5 line-clamp-2">
                  <Markdown
                    className="max-w-none text-sm leading-snug text-muted-foreground"
                    content={feedContent(item)}
                    mentionNames={mentionNames}
                  />
                </div>

                {showDoneAction ? (
                  <Button
                    aria-label={isDone ? "Undo done" : "Mark done"}
                    onClick={() => {
                      if (isDone) {
                        onUndoDone(item.id);
                      } else {
                        onMarkDone(item.id);
                      }
                    }}
                    size="icon"
                    type="button"
                    variant="ghost"
                    className={`pointer-events-auto absolute right-1.5 top-1.5 opacity-0 transition-opacity group-hover:opacity-100 ${isDone ? "text-green-500 opacity-100" : "text-muted-foreground"}`}
                  >
                    <Check />
                  </Button>
                ) : null}
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
}
