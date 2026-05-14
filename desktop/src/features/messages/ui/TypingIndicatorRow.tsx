import * as React from "react";

import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

type TypingIndicatorRowProps = {
  channel: Channel | null;
  className?: string;
  currentPubkey?: string;
  profiles?: UserProfileLookup;
  typingPubkeys: string[];
  variant?: "default" | "activity";
};

function resolveFallbackName(channel: Channel | null, pubkey: string) {
  if (!channel || channel.channelType !== "dm") {
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

function formatTypingLabel(names: string[]) {
  if (names.length === 1) {
    return `${names[0]} is typing...`;
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
  typingPubkeys,
  variant = "default",
}: TypingIndicatorRowProps) {
  const isActivityVariant = variant === "activity";
  const labels = React.useMemo(
    () =>
      typingPubkeys.map((pubkey) =>
        resolveUserLabel({
          pubkey,
          currentPubkey,
          fallbackName: resolveFallbackName(channel, pubkey),
          profiles,
          preferResolvedSelfLabel: true,
        }),
      ),
    [channel, currentPubkey, profiles, typingPubkeys],
  );

  return (
    <div
      aria-live="polite"
      className={cn(
        "shrink-0 bg-transparent",
        isActivityVariant
          ? "flex h-7 items-center px-0 py-0"
          : "px-4 py-2 sm:px-6",
        className,
      )}
      {...(labels.length > 0
        ? { "data-testid": "message-typing-indicator" }
        : {})}
    >
      {labels.length > 0 && (
        <div
          className={cn(
            "flex w-full items-center gap-2",
            isActivityVariant && "h-full",
            !isActivityVariant && "mx-auto max-w-4xl",
          )}
        >
          <div className="flex shrink-0 items-center">
            {typingPubkeys.map((pubkey, index) => {
              const profile = profiles?.[pubkey.toLowerCase()];
              const label = labels[index] ?? pubkey.slice(0, 8);
              return (
                <div
                  key={pubkey}
                  className={cn(
                    "relative shrink-0 rounded-full ring-1 ring-background",
                    isActivityVariant ? "h-[18px] w-[18px]" : "h-5 w-5",
                    index > 0 && "-ml-1.5",
                  )}
                  data-testid="message-typing-avatar"
                >
                  <ProfileAvatar
                    avatarUrl={profile?.avatarUrl ?? null}
                    label={label}
                    className={cn(
                      "rounded-full",
                      isActivityVariant
                        ? "h-[18px] w-[18px] text-[7px]"
                        : "h-5 w-5 text-[8px]",
                    )}
                    iconClassName={
                      isActivityVariant ? "h-2.5 w-2.5" : "h-3 w-3"
                    }
                  />
                </div>
              );
            })}
          </div>
          <p
            className={cn(
              "truncate text-muted-foreground",
              isActivityVariant
                ? "agent-activity-shimmer text-xs font-semibold leading-none"
                : "text-sm",
            )}
            data-testid="message-typing-indicator-label"
          >
            {formatTypingLabel(labels)}
          </p>
        </div>
      )}
    </div>
  );
}
