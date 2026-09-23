import { Check, Copy, Info } from "lucide-react";
import * as React from "react";

import { buildChannelLink } from "@/features/messages/lib/channelLink";
import { buildMessageLink } from "@/features/messages/lib/messageLink";
import { toAppDeepLink } from "@/shared/lib/appDeepLink";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

type ConversationInfoButtonProps = {
  /** Human-readable channel / thread title shown in the panel. */
  title: string;
  channelId: string;
  /** When set, the panel is a thread: include thread-root id + thread deep link. */
  threadRootId?: string | null;
  testId?: string;
};

function CopyableField({
  label,
  displayValue,
  copyValue,
  copySuccessMessage,
  testId,
}: {
  label: string;
  displayValue: string;
  copyValue: string;
  copySuccessMessage: string;
  testId?: string;
}) {
  const [copied, setCopied] = React.useState(false);
  const resetTimer = React.useRef<number | undefined>(undefined);
  React.useEffect(() => () => window.clearTimeout(resetTimer.current), []);

  return (
    <div className="flex min-w-0 items-start gap-1.5" data-testid={testId}>
      <div className="min-w-0 flex-1">
        <div className="text-2xs font-medium text-muted-foreground">
          {label}
        </div>
        <div className="break-all font-mono text-xs">{displayValue}</div>
      </div>
      <Button
        aria-label={`Copy ${label}`}
        onClick={() => {
          copyTextToClipboard(copyValue, copySuccessMessage);
          setCopied(true);
          window.clearTimeout(resetTimer.current);
          resetTimer.current = window.setTimeout(() => setCopied(false), 1500);
        }}
        size="icon-xs"
        type="button"
        variant="ghost"
      >
        {copied ? <Check /> : <Copy />}
      </Button>
    </div>
  );
}

/**
 * Channel / thread header affordance: popover with name + IDs and copy buttons
 * that put full `hulabuzz://` deep links on the clipboard.
 */
export function ConversationInfoButton({
  title,
  channelId,
  threadRootId,
  testId = "conversation-info-button",
}: ConversationInfoButtonProps) {
  const trimmedThreadRootId = threadRootId?.trim() || null;
  const isThread = Boolean(trimmedThreadRootId);
  const channelLink = toAppDeepLink(buildChannelLink(channelId));
  const threadLink = trimmedThreadRootId
    ? toAppDeepLink(
        buildMessageLink({
          channelId,
          messageId: trimmedThreadRootId,
          threadRootId: trimmedThreadRootId,
        }),
      )
    : null;

  return (
    <Popover>
      <Tooltip disableHoverableContent>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <Button
              aria-label={isThread ? "Thread info" : "Channel info"}
              className="shrink-0"
              data-testid={testId}
              size="icon"
              type="button"
              variant="ghost"
            >
              <Info />
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent>
          {isThread ? "Thread info" : "Channel info"}
        </TooltipContent>
      </Tooltip>
      <PopoverContent
        align="end"
        className="w-96 max-w-[90vw] space-y-3"
        data-testid={`${testId}-panel`}
        onOpenAutoFocus={(event) => event.preventDefault()}
      >
        <div>
          <div className="text-2xs font-medium text-muted-foreground">
            {isThread ? "Thread" : "Channel"}
          </div>
          <div className="truncate text-sm font-semibold" title={title}>
            {title}
          </div>
        </div>
        <CopyableField
          copySuccessMessage="Channel link copied to clipboard"
          copyValue={channelLink}
          displayValue={channelId}
          label="Channel ID"
          testId={`${testId}-channel-id`}
        />
        {trimmedThreadRootId && threadLink ? (
          <CopyableField
            copySuccessMessage="Thread link copied to clipboard"
            copyValue={threadLink}
            displayValue={trimmedThreadRootId}
            label="Thread root ID"
            testId={`${testId}-thread-root-id`}
          />
        ) : null}
        <CopyableField
          copySuccessMessage="Link copied to clipboard"
          copyValue={threadLink ?? channelLink}
          displayValue={threadLink ?? channelLink}
          label={isThread ? "Thread link" : "Channel link"}
          testId={`${testId}-link`}
        />
      </PopoverContent>
    </Popover>
  );
}
