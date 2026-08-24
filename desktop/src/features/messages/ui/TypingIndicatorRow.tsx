import * as React from "react";
import { motion, useReducedMotion } from "motion/react";

import {
  COMPOSER_STRIP_AVATAR_MORPH_TRANSITION,
  useComposerStripAvatarLayoutId,
} from "@/features/channels/ui/composerStripAvatarLayout";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Shimmer } from "@/shared/ui/Shimmer";
import { truncatePubkey } from "@/shared/lib/pubkey";

type TypingIndicatorRowProps = {
  channel: Channel | null;
  className?: string;
  currentPubkey?: string;
  /** Extra classes for the "… is typing" label (e.g. weight overrides). */
  labelClassName?: string;
  profiles?: UserProfileLookup;
  typingPubkeys: string[];
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

  return `${names[0]}, ${names[1]}, and ${names.length - 2} more are typing...`;
}

/**
 * One typer's avatar in the overlapping set. Inside a composer activity
 * strip, the avatar carries that strip's shared-element layout id (see
 * composerStripAvatarLayout) so an agent's avatar MORPHS between the merged
 * typing group and its status pill on partition flips instead of exiting
 * one and entering the other. Outside a strip (context null) or under
 * reduced motion the id is dropped and the avatar mounts in place.
 */
function TypingAvatar({
  avatarUrl,
  label,
  overlap,
  pubkey,
}: {
  avatarUrl: string | null;
  label: string;
  overlap: boolean;
  pubkey: string;
}) {
  const shouldReduceMotion = useReducedMotion();
  const morphLayoutId = useComposerStripAvatarLayoutId(pubkey);
  return (
    <motion.div
      className={cn(
        "relative h-5 w-5 shrink-0 rounded-lg ring-1 ring-background",
        overlap && "-ml-1.5",
      )}
      data-testid="message-typing-avatar"
      layoutId={shouldReduceMotion ? undefined : morphLayoutId}
      transition={COMPOSER_STRIP_AVATAR_MORPH_TRANSITION}
    >
      <ProfileAvatar
        avatarUrl={avatarUrl}
        label={label}
        className="h-5 w-5 text-3xs"
        iconClassName="h-4 w-4"
      />
    </motion.div>
  );
}

export function TypingIndicatorRow({
  channel,
  className,
  currentPubkey,
  labelClassName,
  profiles,
  typingPubkeys,
}: TypingIndicatorRowProps) {
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
      className={cn("shrink-0 bg-transparent px-4 py-2 sm:px-6", className)}
      {...(labels.length > 0
        ? { "data-testid": "message-typing-indicator" }
        : {})}
    >
      {labels.length > 0 && (
        <div className="flex min-w-0 w-full items-center gap-2">
          <div className="flex shrink-0 items-center">
            {typingPubkeys.map((pubkey, index) => (
              <TypingAvatar
                key={pubkey}
                avatarUrl={profiles?.[pubkey.toLowerCase()]?.avatarUrl ?? null}
                label={labels[index] ?? truncatePubkey(pubkey)}
                overlap={index > 0}
                pubkey={pubkey}
              />
            ))}
          </div>
          <p
            className={cn(
              "min-w-0 translate-y-px truncate text-xs font-medium leading-4 text-muted-foreground",
              labelClassName,
            )}
            data-testid="message-typing-indicator-label"
          >
            <Shimmer>{formatTypingLabel(labels)}</Shimmer>
          </p>
        </div>
      )}
    </div>
  );
}
