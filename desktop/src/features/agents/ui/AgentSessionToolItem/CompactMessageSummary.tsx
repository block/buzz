import * as React from "react";
import { CheckCheck } from "lucide-react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { cn } from "@/shared/lib/cn";
import { useProfilePanel } from "@/shared/context/ProfilePanelContext";
import { Markdown } from "@/shared/ui/markdown";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { useAgentSessionTranscriptVariant } from "../agentSessionTranscriptContext";
import { MessageLinkHoverCue } from "../activityRenderClasses/MessageLinkHoverCue";
import { TranscriptTimestamp } from "../activityRenderClasses/TranscriptTimestamp";
import { useTranscriptBubbleOverflow } from "../activityRenderClasses/useTranscriptBubbleOverflow";
import { compactSummaryTone } from "./CompactToolSummaryRow";
import type { SentMessageLink } from "./messageLinks";
import { SentMessageContextDialog } from "./SentMessageContextDialog";

export function CompactMessageSummary({
  args,
  avatarUrl,
  description,
  displayName,
  duration,
  hasArgs,
  hasResult,
  isError,
  label,
  messageLink,
  preview,
  pubkey,
  result,
  timestamp,
}: {
  args: Record<string, unknown>;
  avatarUrl: string | null;
  description?: string;
  displayName: string;
  duration: string | null;
  hasArgs: boolean;
  hasResult: boolean;
  isError: boolean;
  label: string;
  messageLink: SentMessageLink | null;
  preview: string | null;
  pubkey: string;
  result: string;
  timestamp: string;
}) {
  const [detailsOpen, setDetailsOpen] = React.useState(false);
  const variant = useAgentSessionTranscriptVariant();
  const { goChannel } = useAppNavigation();
  const { openProfilePanel } = useProfilePanel();
  const isCompactPreview = variant === "compactPreview";
  const shouldClampBubble = !isCompactPreview;
  const [bubbleRef, hasBubbleOverflow] =
    useTranscriptBubbleOverflow(shouldClampBubble);
  const canOpenMessage = shouldClampBubble && messageLink !== null;
  const mutedTone = compactSummaryTone();
  const avatarClassName = cn(
    "mr-2 mt-1 shrink-0",
    isCompactPreview ? "size-5" : "size-7",
  );
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
  const bubbleLinkProps = canOpenMessage
    ? {
        onClick: handleBubbleClick,
        onKeyDown: handleBubbleKeyDown,
        role: "link" as const,
        tabIndex: 0,
      }
    : {};
  return (
    <>
      <div className="flex max-w-full flex-row items-start justify-start">
        {openProfilePanel && !isCompactPreview ? (
          <button
            aria-label={`Open ${displayName} profile`}
            className={cn(
              avatarClassName,
              "pointer-events-auto rounded-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            )}
            onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              openProfilePanel(pubkey);
            }}
            type="button"
          >
            <UserAvatar
              avatarUrl={avatarUrl}
              className="size-full text-xs"
              displayName={displayName}
              size="sm"
              testId="transcript-agent-sent-avatar"
            />
          </button>
        ) : (
          <UserAvatar
            avatarUrl={avatarUrl}
            className={cn(
              avatarClassName,
              isCompactPreview ? "text-3xs" : "text-xs",
            )}
            displayName={displayName}
            size="sm"
            testId="transcript-agent-sent-avatar"
          />
        )}
        <div className="flex min-w-0 flex-1 flex-col items-start gap-1">
          <div
            className={cn(
              "w-full min-w-0 rounded-2xl border px-3 py-2 shadow-sm",
              isCompactPreview
                ? "text-xs leading-4"
                : "text-sm leading-relaxed",
              shouldClampBubble && "relative max-h-36 overflow-hidden",
              canOpenMessage &&
                "group/bubble cursor-pointer transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
              isCompactPreview
                ? isError
                  ? "border-destructive/25 bg-destructive/10 text-destructive"
                  : "border-transparent bg-muted text-foreground"
                : isError
                  ? "border-destructive/25 bg-destructive/10 text-destructive"
                  : "border-transparent bg-muted text-foreground",
              canOpenMessage &&
                (isError ? "hover:bg-destructive/15" : "hover:bg-muted/90"),
            )}
            data-testid="transcript-tool-message-preview"
            ref={bubbleRef}
            {...bubbleLinkProps}
          >
            <Markdown
              className={isCompactPreview ? "text-xs leading-4" : "leading-5"}
              content={preview || "Message content unavailable."}
            />
            {hasBubbleOverflow ? (
              <span
                className={cn(
                  "pointer-events-none absolute inset-x-0 bottom-0 h-8 rounded-b-2xl bg-linear-to-b from-transparent",
                  isError
                    ? "to-destructive/10"
                    : isCompactPreview
                      ? "to-muted"
                      : "to-muted",
                )}
              />
            ) : null}
            {canOpenMessage ? <MessageLinkHoverCue /> : null}
          </div>
          <div className="inline-flex max-w-full items-center gap-1.5 px-1">
            <TranscriptTimestamp
              messageLink={messageLink}
              timestamp={timestamp}
            />
            <button
              aria-label="Show sent message context"
              className={cn(
                "inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-sm transition-colors hover:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
                mutedTone,
              )}
              data-testid="transcript-sent-message-context-button"
              onClick={() => setDetailsOpen(true)}
              title="Show sent message context"
              type="button"
            >
              <CheckCheck className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
      </div>
      <SentMessageContextDialog
        args={args}
        description={description}
        duration={duration}
        hasArgs={hasArgs}
        hasResult={hasResult}
        isError={isError}
        label={label}
        onOpenChange={setDetailsOpen}
        open={detailsOpen}
        preview={preview}
        result={result}
      />
    </>
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
