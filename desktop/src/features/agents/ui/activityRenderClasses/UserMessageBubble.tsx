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
  const isConversation = variant === "conversation";
  // Focus mode shows the whole prompt: the reader is here to read the turn, and
  // the channel-context affordance already lives in the footer, so clamping the
  // bubble would hide the one thing they came for.
  const shouldClampBubble = !isCompactPreview && !isConversation;
  const [bubbleRef, hasBubbleOverflow] =
    useTranscriptBubbleOverflow(shouldClampBubble);
  const text = item.text.trim();
  // The bubble stays a link back to the originating channel message in both
  // polished variants; only the dense preview drops it.
  const messageLink =
    !isCompactPreview && item.channelId && item.messageId
      ? { channelId: item.channelId, messageId: item.messageId }
      : null;
  const authorProfile = item.authorPubkey
    ? profiles?.[item.authorPubkey.toLowerCase()]
    : null;
  // The other variants seed the avatar from `resolveUserLabel(…, fallbackName:
  // item.title)`, whose last resort is the prompt item's title. That title
  // describes the *trigger* that started the turn ("@Mention", "Prompt", "Buzz
  // event"), never a person — harmless when it only picks avatar initials, so
  // this chain is left exactly as it was for `default`/`compactPreview`.
  const triggerSeededLabel = item.authorPubkey
    ? resolveUserLabel({
        pubkey: item.authorPubkey,
        fallbackName: item.title,
        profiles,
      })
    : item.title || "User";
  // Focus mode promotes the author to displayed text, where a trigger title
  // would read as a false identity — an unresolved sender showing up as
  // "@Mention". Identity resolution here therefore stops at the profile
  // (display name, then NIP-05 handle, then the truncated pubkey): a truncated
  // pubkey is a real, if terse, identity, and it keeps the utterance attributed
  // in a full-cover view. `item.title` stays available as trigger chrome in the
  // footer; it is never a name. Only when the item carries no author at all does
  // the row fall back to a generic placeholder.
  const authorLabel = isConversation
    ? item.authorPubkey
      ? resolveUserLabel({ pubkey: item.authorPubkey, profiles })
      : "User"
    : triggerSeededLabel;
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
          // berd caps the user turn at a fixed measure, not a percentage of the
          // column (`--chat-user-message-max-width: 640px`,
          // MessageBubble.tsx:956): a percentage keeps re-wrapping the prompt as
          // the cover width changes, while a fixed measure holds one stable
          // reading line length. `max-w-prompt-bubble` carries the 640px token.
          isConversation && "max-w-prompt-bubble flex-initial",
          className,
        )}
      >
        {isConversation ? (
          <span
            className="px-1 text-xs font-medium text-muted-foreground"
            data-testid="transcript-user-message-author"
          >
            {authorLabel}
          </span>
        ) : null}
        <div
          className={cn(
            "w-full min-w-0 rounded-2xl border border-border/70 bg-transparent p-3 text-sm leading-relaxed text-foreground",
            shouldClampBubble && "relative max-h-36 overflow-hidden",
            messageLink &&
              "group/bubble cursor-pointer transition-colors hover:border-border hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            isCompactPreview && "p-2 text-xs leading-4",
            // berd's user-turn recipe (MessageBubble.tsx:990): a soft tint, no
            // border at all and `px-4 py-2`. Keep Buzz's native chat-bubble
            // `rounded-2xl` radius so prompts and sent-message bubbles share the
            // same silhouette across the channel and focus views.
            // `leading-normal` overrides the `leading-relaxed` base, as berd
            // does, so the prompt sits tighter than the agent's prose.
            isConversation &&
              "relative rounded-2xl border-0 bg-muted/60 px-4 py-2 leading-normal",
            bubbleClassName,
          )}
          ref={bubbleRef}
          {...bubbleLinkProps}
        >
          <Markdown
            className={cn(
              isCompactPreview
                ? "text-xs leading-4"
                : isConversation
                  ? "leading-normal"
                  : "leading-5",
            )}
            content={text || " "}
            mediaInset
          />
          {children}
          {hasBubbleOverflow ? (
            <span className="pointer-events-none absolute inset-x-0 bottom-0 h-8 rounded-b-2xl bg-linear-to-b from-transparent to-background" />
          ) : null}
          {messageLink ? <MessageLinkHoverCue /> : null}
        </div>
        {isConversation && footer ? (
          // Timestamp/context row is chrome, not content: focus mode keeps it
          // out of the reading rhythm until the row is hovered or
          // keyboard-focused. Other variants render `footer` bare so their
          // markup is unchanged.
          <div className="opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
            {footer}
          </div>
        ) : (
          footer
        )}
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
