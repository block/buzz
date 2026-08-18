import * as React from "react";

import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { cn } from "@/shared/lib/cn";
import { useProfilePanel } from "@/shared/context/ProfilePanelContext";
import { Markdown } from "@/shared/ui/markdown";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { useAgentSessionTranscriptVariant } from "../agentSessionTranscriptContext";
import type { TranscriptItem } from "../agentSessionTypes";
import { MessageLinkHoverCue } from "./MessageLinkHoverCue";
import { useTranscriptBubbleOverflow } from "./useTranscriptBubbleOverflow";

export function UserMessageBubble({
  bubbleClassName,
  children,
  className,
  footer,
  item,
  profiles,
}: {
  bubbleClassName?: string;
  children?: React.ReactNode;
  className?: string;
  footer?: React.ReactNode;
  item: Extract<TranscriptItem, { type: "message" }>;
  profiles?: UserProfileLookup;
}) {
  const variant = useAgentSessionTranscriptVariant();
  const { goChannel } = useAppNavigation();
  const { openProfilePanel } = useProfilePanel();
  const isCompactPreview = variant === "compactPreview";
  const shouldClampBubble = !isCompactPreview;
  const [bubbleRef, hasBubbleOverflow] =
    useTranscriptBubbleOverflow(shouldClampBubble);
  const text = item.text.trim();
  const messageLink =
    shouldClampBubble && item.channelId && item.messageId
      ? { channelId: item.channelId, messageId: item.messageId }
      : null;
  const authorProfile = item.authorPubkey
    ? profiles?.[item.authorPubkey.toLowerCase()]
    : null;
  const authorLabel = item.authorPubkey
    ? resolveUserLabel({
        pubkey: item.authorPubkey,
        fallbackName: item.title,
        profiles,
      })
    : item.title || "User";
  const handleBubbleClick = React.useCallback(
    (event: React.MouseEvent<HTMLDivElement>) => {
      if (!messageLink || isNestedInteractiveTarget(event)) return;
      event.preventDefault();
      event.stopPropagation();
      void goChannel(messageLink.channelId, {
        messageId: messageLink.messageId,
      });
    },
    [goChannel, messageLink],
  );
  const handleBubbleKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (
        !messageLink ||
        isNestedInteractiveTarget(event) ||
        (event.key !== "Enter" && event.key !== " ")
      ) {
        return;
      }

      event.preventDefault();
      event.stopPropagation();
      void goChannel(messageLink.channelId, {
        messageId: messageLink.messageId,
      });
    },
    [goChannel, messageLink],
  );
  const bubbleLinkProps = messageLink
    ? {
        onClick: handleBubbleClick,
        onKeyDown: handleBubbleKeyDown,
        role: "link" as const,
        tabIndex: 0,
      }
    : {};

  return (
    <div
      className={cn(
        "flex flex-row items-start animate-in fade-in duration-200 motion-reduce:animate-none",
        isCompactPreview ? "justify-start" : "justify-end",
      )}
      data-role="user-message"
      data-testid="transcript-user-message"
    >
      {isCompactPreview ? null : item.authorPubkey && openProfilePanel ? (
        <button
          aria-label={`Open ${authorLabel} profile`}
          className="pointer-events-auto order-last ml-2 mt-1 size-7 shrink-0 rounded-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          onClick={(event) => {
            event.preventDefault();
            event.stopPropagation();
            if (item.authorPubkey) {
              openProfilePanel(item.authorPubkey);
            }
          }}
          type="button"
        >
          <UserAvatar
            avatarUrl={authorProfile?.avatarUrl ?? null}
            className="size-full text-xs"
            displayName={authorLabel}
            size="sm"
          />
        </button>
      ) : (
        <UserAvatar
          avatarUrl={authorProfile?.avatarUrl ?? null}
          className="order-last ml-2 mt-1 size-7 shrink-0 text-xs"
          displayName={authorLabel}
          size="sm"
        />
      )}
      <div
        className={cn(
          "group relative flex min-w-0 flex-1 flex-col items-end gap-1",
          isCompactPreview && "items-start",
          className,
        )}
      >
        <div
          className={cn(
            "w-full min-w-0 rounded-2xl border border-border/70 bg-transparent p-3 text-sm leading-relaxed text-foreground",
            shouldClampBubble && "relative max-h-36 overflow-hidden",
            messageLink &&
              "group/bubble cursor-pointer transition-colors hover:border-border hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            isCompactPreview && "p-2 text-xs leading-4",
            bubbleClassName,
          )}
          ref={bubbleRef}
          {...bubbleLinkProps}
        >
          <Markdown
            className={isCompactPreview ? "text-xs leading-4" : "leading-5"}
            content={text || " "}
            mediaInset
          />
          {children}
          {hasBubbleOverflow ? (
            <span className="pointer-events-none absolute inset-x-0 bottom-0 h-8 rounded-b-2xl bg-linear-to-b from-transparent to-background" />
          ) : null}
          {messageLink ? <MessageLinkHoverCue /> : null}
        </div>
        {footer}
      </div>
    </div>
  );
}

function isNestedInteractiveTarget(
  event: React.MouseEvent<HTMLElement> | React.KeyboardEvent<HTMLElement>,
) {
  const target =
    event.target instanceof Element
      ? event.target.closest(
          "a,button,input,select,textarea,summary,[role='button'],[role='link']",
        )
      : null;

  return target !== null && target !== event.currentTarget;
}
