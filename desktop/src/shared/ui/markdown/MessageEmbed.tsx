import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { LockKeyhole } from "lucide-react";

import { useUserProfileQuery } from "@/features/profile/hooks";
import {
  canReadMessageEmbedSource,
  isMatchingMessageEmbedEvent,
  messageEmbedExcerpt,
  type MessageEmbedLink,
} from "@/features/messages/lib/messageEmbed";
import { getEventById } from "@/shared/api/tauri";
import type { Channel } from "@/shared/api/types";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { UserAvatar } from "@/shared/ui/UserAvatar";

export function MessageEmbed({
  channel,
  link,
  onOpen,
}: {
  channel: Channel | undefined;
  link: MessageEmbedLink;
  onOpen: () => void;
}) {
  const canRead = canReadMessageEmbedSource(channel);
  const eventQuery = useQuery({
    queryKey: ["message-embed", link.channelId, link.messageId],
    queryFn: () => getEventById(link.messageId),
    enabled: canRead,
    retry: false,
    staleTime: 60_000,
  });
  const event =
    canRead &&
    eventQuery.data &&
    isMatchingMessageEmbedEvent(eventQuery.data, link)
      ? eventQuery.data
      : null;
  // Never resolve an author profile until channel access and event-channel
  // integrity have both been established.
  const profileQuery = useUserProfileQuery(event?.pubkey);
  const profile = profileQuery.data;
  const excerpt = event
    ? messageEmbedExcerpt(event.content) || "Message has no text"
    : "";
  const [expanded, setExpanded] = useState(false);
  const [isClamped, setIsClamped] = useState(false);
  const excerptRef = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    const excerptElement = excerptRef.current;
    if (!excerptElement) return;
    const updateClampedState = () =>
      setIsClamped(excerptElement.scrollHeight > excerptElement.clientHeight);
    updateClampedState();
    const observer = new ResizeObserver(updateClampedState);
    observer.observe(excerptElement);
    return () => observer.disconnect();
  });

  if (!canRead || eventQuery.isError || (eventQuery.data && !event)) {
    return (
      <div
        className="flex w-full max-w-2xl items-center gap-3 border-l-[3px] border-border py-1 pl-3 text-muted-foreground/70"
        data-message-embed="unavailable"
      >
        <LockKeyhole aria-hidden="true" className="h-4 w-4 shrink-0" />
        <span className="text-sm font-medium">
          Private or unavailable message
        </span>
      </div>
    );
  }

  if (!event) {
    return (
      <div
        className="w-full max-w-2xl animate-pulse border-l-[3px] border-border py-1 pl-3"
        data-message-embed="loading"
      >
        <span className="sr-only">Loading message preview</span>
        <div className="mb-1 h-4 w-36 rounded bg-muted" />
        <div className="h-4 w-full rounded bg-muted" />
        <div className="mt-1 h-4 w-2/3 rounded bg-muted" />
      </div>
    );
  }

  const displayName =
    profile?.displayName?.trim() || truncatePubkey(event.pubkey);

  return (
    <div
      className="w-full max-w-2xl border-l-[3px] border-border py-1 pl-3"
      data-message-embed="resolved"
    >
      <div className="mb-0.5 flex min-w-0 items-center gap-2">
        <UserAvatar
          avatarUrl={profile?.avatarUrl ?? null}
          className="shrink-0"
          displayName={displayName}
          fallbackDelayMs={0}
          size="xs"
        />
        <span className="min-w-0 truncate text-xs font-semibold text-foreground">
          {displayName}
        </span>
        <span aria-hidden="true" className="text-xs text-muted-foreground">
          ·
        </span>
        <button
          aria-label={`Open message by ${displayName} in ${channel?.name ?? "channel"}`}
          className="min-w-0 truncate text-xs text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          onClick={onOpen}
          type="button"
        >
          #{channel?.name}
        </button>
      </div>
      <span
        className={
          expanded
            ? "block text-sm leading-5 text-foreground"
            : "line-clamp-3 text-sm leading-5 text-foreground"
        }
        ref={excerptRef}
      >
        {excerpt}
      </span>
      {expanded || isClamped ? (
        <button
          className="mt-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          onClick={() => setExpanded((value) => !value)}
          type="button"
        >
          {expanded ? "Show less" : "Show more"}
        </button>
      ) : null}
    </div>
  );
}
