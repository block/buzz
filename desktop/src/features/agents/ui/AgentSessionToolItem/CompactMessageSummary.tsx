import * as React from "react";
import { CheckCheck } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { TranscriptTimestamp } from "../activityRenderClasses/TranscriptTimestamp";
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
  result: string;
  timestamp: string;
}) {
  const [detailsOpen, setDetailsOpen] = React.useState(false);
  const mutedTone = compactSummaryTone();
  return (
    <>
      <div className="flex max-w-full flex-row items-start justify-start">
        <UserAvatar
          avatarUrl={avatarUrl}
          className="mr-2 mt-1 shrink-0"
          displayName={displayName}
          size="xs"
          testId="transcript-agent-sent-avatar"
        />
        <div className="flex max-w-[85%] min-w-0 flex-col items-start gap-1">
          <div
            className={cn(
              "min-w-0 rounded-2xl border px-3 py-2 text-sm leading-relaxed shadow-sm",
              isError
                ? "border-destructive/25 bg-destructive/10 text-destructive"
                : "border-primary/15 bg-primary/6 text-foreground",
            )}
            data-testid="transcript-tool-message-preview"
          >
            <p className="whitespace-pre-wrap wrap-break-word">
              {preview || "Message content unavailable."}
            </p>
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
