import type * as React from "react";

import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { Markdown } from "@/shared/ui/markdown";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { useAgentSessionTranscriptVariant } from "../agentSessionTranscriptContext";
import type { TranscriptItem } from "../agentSessionTypes";

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
  const isCompactPreview = variant === "compactPreview";
  const text = item.text.trim();
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

  return (
    <div
      className={cn(
        "flex flex-row items-start animate-in fade-in duration-200 motion-reduce:animate-none",
        isCompactPreview ? "justify-start" : "justify-end",
      )}
      data-role="user-message"
      data-testid="transcript-user-message"
    >
      {isCompactPreview ? null : (
        <UserAvatar
          avatarUrl={authorProfile?.avatarUrl ?? null}
          className="order-last ml-2 mt-1 shrink-0"
          displayName={authorLabel}
          size="xs"
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
            "w-full min-w-0 rounded-2xl bg-muted p-3 text-sm leading-relaxed text-foreground",
            isCompactPreview &&
              "rounded-none bg-transparent p-0 text-xs leading-4",
            bubbleClassName,
          )}
        >
          <Markdown
            className={isCompactPreview ? "text-xs leading-4" : undefined}
            content={text || " "}
            mediaInset
          />
          {children}
        </div>
        {footer}
      </div>
    </div>
  );
}
