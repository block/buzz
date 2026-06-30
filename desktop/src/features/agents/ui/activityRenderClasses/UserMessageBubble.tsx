import type * as React from "react";

import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { Markdown } from "@/shared/ui/markdown";
import { UserAvatar } from "@/shared/ui/UserAvatar";
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
      className="flex flex-row items-start justify-end animate-in fade-in duration-200 motion-reduce:animate-none"
      data-role="user-message"
      data-testid="transcript-user-message"
    >
      <UserAvatar
        avatarUrl={authorProfile?.avatarUrl ?? null}
        className="order-last ml-2 mt-1 shrink-0"
        displayName={authorLabel}
        size="xs"
      />
      <div
        className={cn(
          "group relative flex max-w-[85%] min-w-0 flex-col items-end gap-1",
          className,
        )}
      >
        <div
          className={cn(
            "w-full min-w-0 rounded-2xl bg-muted p-3 text-sm leading-relaxed text-foreground",
            bubbleClassName,
          )}
        >
          <Markdown content={text || " "} mediaInset tight />
          {children}
        </div>
        {footer}
      </div>
    </div>
  );
}
