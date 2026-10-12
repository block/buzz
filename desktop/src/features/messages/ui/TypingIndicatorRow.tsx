import * as React from "react";

import type { TypingIndicatorEntry } from "@/features/messages/useChannelTyping";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Shimmer } from "@/shared/ui/Shimmer";
import { truncateNpub } from "@/shared/lib/pubkey";

type TypingIndicatorRowProps = {
  channel: Channel | null;
  className?: string;
  currentPubkey?: string;
  profiles?: UserProfileLookup;
  typingEntries: readonly TypingIndicatorEntry[];
  variant?: "default" | "activity";
};

function resolveFallbackName(channel: Channel | null, pubkey: string) {
  if (channel?.channelType !== "dm") {
    return null;
  }

  const participantIndex = channel.participantPubkeys.findIndex(
    (candidate) => candidate.toLowerCase() === pubkey.toLowerCase(),
  );

  if (participantIndex < 0) {
    return null;
  }

  return channel.participants[participantIndex] ?? null;
}

/**
 * The typing row's text. A single typer's activity label (carried by the
 * typing event's `content`) follows the name; with several typers the row
 * stays generic — per-typer labels would not fit.
 */
function formatTypingLabel(names: string[], activityLabel: string | null) {
  if (names.length === 1) {
    return activityLabel
      ? `${names[0]} is typing — ${activityLabel}`
      : `${names[0]} is typing...`;
  }

  if (names.length === 2) {
    return `${names[0]} and ${names[1]} are typing...`;
  }

  if (names.length === 3) {
    return `${names[0]}, ${names[1]}, and ${names[2]} are typing...`;
  }

  return `${names[0]}, ${names[1]}, and ${names.length - 2} others are typing...`;
}

export function TypingIndicatorRow({
  channel,
  className,
  currentPubkey,
  profiles,
  typingEntries,
  variant = "default",
}: TypingIndicatorRowProps) {
  const isActivityVariant = variant === "activity";
  const labels = React.useMemo(
    () =>
      typingEntries.map((entry) =>
        resolveUserLabel({
          pubkey: entry.pubkey,
          currentPubkey,
          fallbackName: resolveFallbackName(channel, entry.pubkey),
          profiles,
          preferResolvedSelfLabel: true,
        }),
      ),
    [channel, currentPubkey, profiles, typingEntries],
  );
  const activityLabel =
    typingEntries.length === 1 ? (typingEntries[0]?.label ?? null) : null;

  return (
    <div
      aria-live="polite"
      className={cn(
        "shrink-0 bg-transparent",
        isActivityVariant ? "flex items-center px-0 py-0" : "px-4 py-2 sm:px-6",
        className,
      )}
      {...(labels.length > 0
        ? { "data-testid": "message-typing-indicator" }
        : {})}
    >
      {labels.length > 0 && (
        <div
          className={cn(
            "flex min-w-0 w-full items-center",
            isActivityVariant ? "h-full gap-1.5" : "gap-2",
          )}
        >
          <div className="flex shrink-0 items-center">
            {typingEntries.map((entry, index) => {
              const profile = profiles?.[entry.pubkey.toLowerCase()];
              const label = labels[index] ?? truncateNpub(entry.pubkey);
              return (
                <div
                  key={entry.pubkey}
                  className={cn(
                    "relative shrink-0 ring-1 ring-background",
                    profile?.isAgent ? "rounded-squircle" : "rounded-full",
                    isActivityVariant ? "h-4 w-4" : "h-5 w-5",
                    index > 0 && "-ml-1.5",
                  )}
                  data-testid="message-typing-avatar"
                >
                  <ProfileAvatar
                    avatarUrl={profile?.avatarUrl ?? null}
                    label={label}
                    className={cn(
                      isActivityVariant
                        ? "h-4 w-4 text-3xs"
                        : "h-5 w-5 text-3xs",
                    )}
                    iconClassName={
                      isActivityVariant ? "h-2.5 w-2.5" : "h-4 w-4"
                    }
                    shape={profile?.isAgent ? "squircle" : "circle"}
                  />
                </div>
              );
            })}
          </div>
          <p
            className={cn(
              "min-w-0 translate-y-px truncate text-muted-foreground",
              isActivityVariant
                ? "text-2xs font-medium leading-3"
                : "text-xs font-medium leading-4",
            )}
            data-testid="message-typing-indicator-label"
          >
            <Shimmer>{formatTypingLabel(labels, activityLabel)}</Shimmer>
          </p>
        </div>
      )}
    </div>
  );
}
