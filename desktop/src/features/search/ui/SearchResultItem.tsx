import type * as React from "react";
import {
  ArrowRight,
  Bot,
  FileText,
  Hash,
  MessageCircle,
  Plus,
  User,
  type LucideIcon,
} from "lucide-react";

import { HashSearch } from "@/shared/ui/icons";
import {
  resolveUserLabel,
  resolveUserSecondaryLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import type { Channel, SearchHit, UserSearchResult } from "@/shared/api/types";
import { Badge } from "@/shared/ui/badge";
import { UserAvatar } from "@/shared/ui/UserAvatar";

export type SearchResult =
  | {
      kind: "action";
      action: {
        description?: string;
        id: "browse-channels" | "create-agent" | "create-channel";
        title: string;
      };
    }
  | { kind: "channel"; channel: Channel }
  | { kind: "user"; user: UserSearchResult }
  | { kind: "message"; hit: SearchHit };

export function resultKey(result: SearchResult) {
  if (result.kind === "action") {
    return `action-${result.action.id}`;
  }

  if (result.kind === "channel") {
    return `channel-${result.channel.id}`;
  }

  if (result.kind === "user") {
    return `user-${result.user.pubkey}`;
  }

  return `message-${result.hit.eventId}`;
}

export function resultTestId(result: SearchResult) {
  if (result.kind === "action") {
    return `search-result-action-${result.action.id}`;
  }

  if (result.kind === "channel") {
    return `search-result-channel-${result.channel.id}`;
  }

  if (result.kind === "user") {
    return `search-result-user-${result.user.pubkey}`;
  }

  return `search-result-${result.hit.eventId}`;
}

export function resultIcon(
  result: SearchResult,
  channelLookup: ReadonlyMap<string, Channel>,
) {
  if (result.kind === "action") {
    if (result.action.id === "browse-channels") {
      return HashSearch;
    }
    return result.action.id === "create-agent" ? Bot : Plus;
  }

  if (result.kind === "user") {
    return result.user.isAgent ? Bot : User;
  }

  const channelType =
    result.kind === "channel"
      ? result.channel.channelType
      : result.hit.channelId
        ? channelLookup.get(result.hit.channelId)?.channelType
        : undefined;

  if (channelType === "forum") {
    return FileText;
  }

  if (channelType === "dm") {
    return MessageCircle;
  }

  return Hash;
}

export function SearchResultShell({
  children,
  icon: Icon,
  isSelected,
  onClick,
  onMouseEnter,
  testId,
}: {
  children: React.ReactNode;
  icon: LucideIcon;
  isSelected: boolean;
  onClick: () => void;
  onMouseEnter: () => void;
  testId: string;
}) {
  return (
    <button
      className={
        isSelected
          ? "w-full rounded-2xl border border-primary/30 bg-primary/10 px-4 py-4 text-left shadow-xs outline-hidden transition-colors"
          : "w-full rounded-2xl border border-border/80 bg-card/60 px-4 py-4 text-left shadow-xs outline-hidden transition-colors hover:border-primary/20 hover:bg-accent"
      }
      data-testid={testId}
      onClick={onClick}
      onMouseEnter={onMouseEnter}
      type="button"
    >
      <div className="flex items-start gap-3">
        <div className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-2xl bg-secondary text-secondary-foreground">
          <Icon className="h-4 w-4" />
        </div>

        {children}

        <ArrowRight className="mt-1 h-4 w-4 shrink-0 text-muted-foreground" />
      </div>
    </button>
  );
}

export function ChannelResultBody({ channel }: { channel: Channel }) {
  return (
    <div className="min-w-0 flex-1">
      <div className="flex flex-wrap items-center gap-2">
        <p className="text-sm font-semibold tracking-tight">{channel.name}</p>
        <Badge variant="secondary">{channel.channelType}</Badge>
        <p className="ml-auto whitespace-nowrap text-xs text-muted-foreground">
          Channel
        </p>
      </div>
      {channel.description ? (
        <p className="mt-2 text-sm leading-6 text-foreground">
          {channel.description}
        </p>
      ) : null}
    </div>
  );
}

function describeSearchHit(hit: SearchHit) {
  switch (hit.kind) {
    case 1:
      return "Note";
    case 45001:
      return "Forum post";
    case 45003:
      return "Forum reply";
    case 43001:
      return "Agent job";
    case 43003:
      return "Agent update";
    case 46010:
      return "Approval request";
    default:
      return "Message";
  }
}

function truncateContent(content: string) {
  const trimmed = content.trim();
  if (trimmed.length === 0) {
    return "No message body.";
  }

  if (trimmed.length <= 180) {
    return trimmed;
  }

  return `${trimmed.slice(0, 177)}...`;
}

function formatRelativeTime(unixSeconds: number) {
  const diff = Math.floor(Date.now() / 1_000) - unixSeconds;

  if (diff < 60) {
    return "just now";
  }

  if (diff < 60 * 60) {
    return `${Math.floor(diff / 60)}m ago`;
  }

  if (diff < 60 * 60 * 24) {
    return `${Math.floor(diff / (60 * 60))}h ago`;
  }

  return new Intl.DateTimeFormat("en-US", {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(new Date(unixSeconds * 1_000));
}

export function MessageResultBody({
  currentPubkey,
  hit,
  resultProfiles,
}: {
  currentPubkey?: string;
  hit: SearchHit;
  resultProfiles?: UserProfileLookup;
}) {
  const authorLabel = resolveUserLabel({
    pubkey: hit.pubkey,
    currentPubkey,
    profiles: resultProfiles,
    preferResolvedSelfLabel: true,
  });
  const authorSecondaryLabel = resolveUserSecondaryLabel({
    pubkey: hit.pubkey,
    profiles: resultProfiles,
  });
  const avatarUrl =
    resultProfiles?.[hit.pubkey.toLowerCase()]?.avatarUrl ?? null;

  return (
    <div className="min-w-0 flex-1">
      <div className="flex flex-wrap items-center gap-2">
        <p className="text-sm font-semibold tracking-tight">
          {hit.channelName}
        </p>
        <Badge variant="secondary">{describeSearchHit(hit)}</Badge>
        <span className="flex items-center gap-1 text-xs text-muted-foreground">
          <UserAvatar
            avatarUrl={avatarUrl}
            displayName={authorLabel}
            size="xs"
          />
          {authorLabel}
        </span>
        <p className="ml-auto whitespace-nowrap text-xs text-muted-foreground">
          {formatRelativeTime(hit.createdAt)}
        </p>
      </div>
      {authorSecondaryLabel ? (
        <p className="mt-1 text-xs text-muted-foreground">
          {authorSecondaryLabel}
        </p>
      ) : null}
      <p className="mt-2 text-sm leading-6 text-foreground">
        {truncateContent(hit.content)}
      </p>
    </div>
  );
}
