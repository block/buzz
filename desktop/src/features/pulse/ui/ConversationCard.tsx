import {
  ArrowUpRight,
  Bot,
  Globe2,
  Hash,
  LockKeyhole,
  MessageCircle,
} from "lucide-react";
import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useCommunities } from "@/features/communities/useCommunities";
import { ForumComposer } from "@/features/forum/ui/ForumComposer";
import { splitOutgoingTags } from "@/features/messages/lib/imetaMediaMarkdown";
import type { TimelineMessage } from "@/features/messages/types";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { usePublishNoteMutation } from "@/features/pulse/hooks";
import type { PulseConversation } from "@/features/pulse/lib/unifiedFeed";
import { sendChannelMessage } from "@/shared/api/tauri";
import type { UserProfileSummary } from "@/shared/api/types";
import { Markdown } from "@/shared/ui/markdown";
import { UserAvatar } from "@/shared/ui/UserAvatar";

import { MessageBubbleContext } from "@/features/messages/ui/MessageBubbleContext";
import { MessageBubbleLayout } from "@/features/messages/ui/MessageBubbleLayout";
import {
  hasSameMessageAuthor,
  isWithinGroupingWindow,
} from "@/features/messages/lib/messageGrouping";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { resolveMentionProps } from "@/shared/lib/resolveMentionNames";
import { parseImetaTags } from "@/shared/ui/markdown/parseImeta";

function relativeTime(time: number) {
  const minutes = Math.max(0, Math.floor((Date.now() / 1000 - time) / 60));
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes}m`;
  if (minutes < 1440) return `${Math.floor(minutes / 60)}h`;
  return new Date(time * 1000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

function FeedMessage({
  message,
  continuation = false,
  context,
  summary,
  currentPubkey,
  profiles,
  footer,
}: {
  message: TimelineMessage;
  continuation?: boolean;
  context?: React.ReactNode;
  summary?: boolean;
  currentPubkey?: string;
  profiles: Record<string, UserProfileSummary>;
  footer?: React.ReactNode;
}) {
  const outgoing = Boolean(
    currentPubkey &&
      message.pubkey &&
      normalizePubkey(currentPubkey) === normalizePubkey(message.pubkey),
  );
  const mentionProps = resolveMentionProps(
    message.tags,
    profiles,
    message.body,
  );
  const avatar = (
    <UserProfilePopover
      pubkey={message.pubkey ?? ""}
      role={message.isAgent ? "bot" : undefined}
      triggerAriaLabel={`Open ${message.author}'s profile`}
    >
      <span className="relative z-10 h-fit shrink-0 rounded-full bg-background focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring">
        <UserAvatar
          avatarUrl={message.avatarUrl ?? null}
          displayName={message.author}
          fallbackDelayMs={0}
          className={summary ? "!h-12 !w-12" : "!h-7 !w-7"}
          shape={message.isAgent ? "squircle" : "circle"}
        />
      </span>
    </UserProfilePopover>
  );
  const header = (
    <div
      className={`flex flex-wrap items-center gap-x-1.5 gap-y-1 ${summary ? "text-message" : "text-message-timestamp"}`}
    >
      <span className="font-semibold text-foreground">{message.author}</span>
      {message.isAgent && (
        <span className="text-muted-foreground" title="Agent">
          <Bot aria-hidden className="h-3 w-3" />
          <span className="sr-only">Agent</span>
        </span>
      )}
      {context}
      <span aria-hidden className="text-muted-foreground/60">
        ·
      </span>
      <time
        className="text-muted-foreground"
        dateTime={new Date(message.createdAt * 1000).toISOString()}
        title={`${new Date(message.createdAt * 1000).toLocaleString()}${message.edited ? " · Edited" : ""}`}
      >
        {relativeTime(message.createdAt)}
      </time>
    </div>
  );
  if (summary)
    return (
      <div className="relative flex gap-3">
        {avatar}
        <div className="min-w-0 flex-1 self-center">{header}</div>
      </div>
    );
  return (
    <MessageBubbleContext.Provider value={currentPubkey ?? ""}>
      <div className={`relative flex gap-2.5 ${outgoing ? "justify-end" : ""}`}>
        <MessageBubbleLayout
          outgoing={outgoing}
          continuation={continuation}
          avatar={avatar}
          header={header}
          metadata={header}
          extras={null}
          footer={footer}
          body={
            <Markdown
              content={message.body}
              {...mentionProps}
              imetaByUrl={
                message.tags ? parseImetaTags(message.tags) : undefined
              }
            />
          }
        />
      </div>
    </MessageBubbleContext.Provider>
  );
}

export function ConversationCard({
  item,
  currentPubkey,
  profiles,
  onRefresh,
  summary,
  divider = false,
  onOpenContext,
}: {
  item: PulseConversation;
  currentPubkey?: string;
  profiles: Record<string, UserProfileSummary>;
  onRefresh: () => void;
  summary?: string;
  divider?: boolean;
  onOpenContext?: () => void;
}) {
  const [expanded, setExpanded] = React.useState(false);
  const [replying, setReplying] = React.useState(false);
  const [sending, setSending] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const { activeCommunity } = useCommunities();
  const queryClient = useQueryClient();
  const publish = usePublishNoteMutation(currentPubkey);
  const { goChannel, goForumPost } = useAppNavigation();
  const head =
    item.messages.find((m) => m.id === item.rootId) ?? item.messages[0];
  const replies = item.messages.filter((m) => m.id !== head.id);
  const visibleReplies = expanded ? replies : [];
  const isDm = item.channel?.channelType === "dm";
  const SourceIcon = item.isPrivate
    ? LockKeyhole
    : item.channel
      ? Hash
      : Globe2;
  const sourceLabel = isDm
    ? `DM · ${item.channel?.name}`
    : item.channel
      ? `${item.channel.visibility === "private" ? "Private channel · " : ""}#${item.channel.name}`
      : "Public note";
  const destination = isDm
    ? `Reply privately in ${item.channel?.name}`
    : item.channel
      ? `Reply in #${item.channel.name}`
      : "Reply publicly";
  const openContext = () => {
    if (onOpenContext) {
      onOpenContext();
      return;
    }
    if (!item.channel) return;
    if (item.channel.channelType === "forum")
      void goForumPost(item.channel.id, item.rootId);
    else
      void goChannel(item.channel.id, {
        messageId: head.id,
        threadRootId: item.rootId,
        thread: item.rootId,
      });
  };
  const submit = async (
    content: string,
    mentions: string[],
    tags?: string[][],
  ) => {
    setSending(true);
    setError(null);
    try {
      if (item.channel) {
        const { mediaTags, emojiTags, mentionTags } = splitOutgoingTags(tags);
        await sendChannelMessage(
          item.channel.id,
          content,
          head.id,
          mediaTags,
          mentions,
          item.channel.channelType === "forum" ? 45003 : undefined,
          emojiTags,
          mentionTags,
          undefined,
          undefined,
          activeCommunity?.relayUrl,
          currentPubkey,
          item.rootId,
        );
        void queryClient.invalidateQueries({
          queryKey: ["channel-messages", item.channel.id],
        });
      } else {
        await publish.mutateAsync({
          content,
          replyTo: head.id,
          mentionPubkeys: [
            ...new Set([head.pubkey ?? "", ...mentions].filter(Boolean)),
          ],
          mediaTags: tags,
        });
      }
      setReplying(false);
      onRefresh();
    } catch (failure) {
      setError(
        failure instanceof Error
          ? failure.message
          : "Could not send your reply. Try again.",
      );
      throw failure;
    } finally {
      setSending(false);
    }
  };
  const replyToggle =
    replies.length > 0 ? (
      <button
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded(!expanded)}
        className="my-3 flex items-center gap-2 rounded text-xs font-medium text-muted-foreground hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
      >
        {expanded
          ? "Hide replies"
          : `View ${replies.length} ${replies.length === 1 ? "reply" : "replies"}`}
      </button>
    ) : null;
  const actions = (
    <div
      className={`flex items-center gap-5 text-xs text-muted-foreground ${summary ? "mt-5" : "mt-2"}`}
    >
      <button
        type="button"
        aria-expanded={replying}
        aria-label="Reply"
        title="Reply"
        onClick={() => setReplying(!replying)}
        className="inline-flex h-8 min-w-8 items-center justify-center gap-2 rounded-full px-2 hover:bg-muted/50 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
      >
        <MessageCircle aria-hidden className="h-4 w-4" />
        {summary ? (
          <span>Reply</span>
        ) : (
          replies.length > 0 && <span>{replies.length}</span>
        )}
      </button>
      {(item.channel || onOpenContext) && (
        <button
          type="button"
          onClick={openContext}
          aria-label="Open conversation"
          title="Open conversation"
          className="ml-auto inline-flex h-8 min-w-8 items-center justify-center gap-2 rounded-full px-2 hover:bg-muted/50 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
        >
          {summary && <span>View conversation</span>}
          <ArrowUpRight aria-hidden className="h-3.5 w-3.5" />
        </button>
      )}
    </div>
  );
  return (
    <article
      data-testid={summary ? "pulse-briefing-highlight" : "pulse-conversation"}
      data-conversation-id={item.id}
      className={`px-5 sm:px-7 ${summary || divider ? "border-b border-border/50" : ""} ${summary ? "py-6" : "py-[12px]"}`}
    >
      {!summary && head.id !== item.rootId && (
        <p className="mb-3 pl-12 text-xs text-muted-foreground">
          <button
            type="button"
            className="underline underline-offset-2"
            onClick={openContext}
          >
            Open earlier context
          </button>
        </p>
      )}
      <FeedMessage
        message={head}
        summary={Boolean(summary)}
        currentPubkey={currentPubkey}
        profiles={profiles}
        footer={
          !summary ? (
            <>
              {replyToggle}
              {actions}
            </>
          ) : undefined
        }
        context={
          item.channel ? (
            <button
              type="button"
              onClick={openContext}
              aria-label={sourceLabel}
              title={sourceLabel}
              className="inline-flex min-w-0 max-w-64 items-center gap-1 rounded text-muted-foreground hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
            >
              {item.isPrivate && (
                <LockKeyhole aria-hidden className="h-3 w-3 shrink-0" />
              )}
              <span className="truncate">
                {isDm ? item.channel.name : `#${item.channel.name}`}
              </span>
            </button>
          ) : (
            <span title="Public note" className="text-muted-foreground">
              <Globe2 aria-hidden className="h-3 w-3" />
              <span className="sr-only">Public note</span>
            </span>
          )
        }
      />
      {summary && (
        <p className="mt-4 break-words text-xl leading-relaxed tracking-tight">
          {summary}
        </p>
      )}
      {summary && replyToggle}
      {visibleReplies.length > 0 && (
        <div className="mt-[24px]">
          {visibleReplies.map((message, index) => {
            const previous = visibleReplies[index - 1];
            const continuation =
              hasSameMessageAuthor(previous, message) &&
              isWithinGroupingWindow(previous?.createdAt, message.createdAt);
            return (
              <div
                key={message.id}
                className={
                  index === 0
                    ? undefined
                    : continuation
                      ? "mt-[2px]"
                      : "mt-[24px]"
                }
              >
                <FeedMessage
                  message={message}
                  continuation={continuation}
                  currentPubkey={currentPubkey}
                  profiles={profiles}
                />
              </div>
            );
          })}
        </div>
      )}
      {summary && actions}
      {replying && (
        <div
          className={`mt-4 rounded-xl border border-border/60 bg-muted/15 p-3 ${summary ? "" : "ml-12"}`}
        >
          <p className="mb-3 flex items-center gap-1.5 text-xs font-medium">
            <SourceIcon aria-hidden className="h-3 w-3" />
            {destination}
          </p>
          {error && (
            <p role="alert" className="mb-2 text-xs text-destructive">
              {error}
            </p>
          )}
          <ForumComposer
            channelId={item.channel?.id}
            channelType={item.channel?.channelType}
            draftKey={item.channel ? `thread:${item.rootId}` : undefined}
            className="border-0 bg-transparent p-0 shadow-none"
            placeholder={destination}
            isSending={sending}
            disabled={!currentPubkey}
            onSubmit={submit}
            onCancel={() => setReplying(false)}
            profiles={profiles}
          />
        </div>
      )}
    </article>
  );
}
