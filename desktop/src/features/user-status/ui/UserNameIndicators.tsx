import * as React from "react";

import { useIsUserInHuddle } from "@/features/huddle/HuddlePresenceContext";
import { useUserStatusLookupContext } from "@/features/user-status/UserStatusLookupContext";
import {
  useUserStatusQuery,
  visibleUserStatus,
} from "@/features/user-status/hooks";
import {
  DEFAULT_USER_STATUS_EMOJI,
  StatusEmoji,
} from "@/features/user-status/ui/StatusEmoji";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

export function UserNameIndicators({
  className,
  pubkey,
  size = "chat",
}: {
  className?: string;
  pubkey: string | undefined;
  size?: "chat" | "dm";
}) {
  const normalizedPubkey = normalizePubkey(pubkey ?? "");
  const sharedStatus = useUserStatusLookupContext();
  const registerStatus = sharedStatus?.register;
  React.useEffect(
    () => registerStatus?.(normalizedPubkey),
    [normalizedPubkey, registerStatus],
  );
  const fallbackStatusQuery = useUserStatusQuery(
    !sharedStatus && normalizedPubkey ? [normalizedPubkey] : [],
  );
  const cachedStatus = normalizedPubkey
    ? (sharedStatus?.lookup[normalizedPubkey] ??
      fallbackStatusQuery.data?.[normalizedPubkey])
    : null;
  const status = visibleUserStatus(cachedStatus);
  const isInHuddle = useIsUserInHuddle(normalizedPubkey);
  const statusEmoji = status?.emoji || DEFAULT_USER_STATUS_EMOJI;
  const indicatorTextClass =
    size === "dm" ? "text-status-indicator" : "text-sm";
  const statusEmojiClass = size === "dm" ? "size-status-indicator" : "size-3.5";

  if (!status && !isInHuddle) return null;

  return (
    <span
      className={cn("inline-flex shrink-0 items-center gap-0.5", className)}
      data-testid="user-name-indicators"
    >
      {isInHuddle ? (
        <Tooltip disableHoverableContent>
          <TooltipTrigger asChild>
            <span
              aria-label="🎧 In a huddle"
              className={cn(
                "inline-flex cursor-default items-center justify-center leading-none",
                indicatorTextClass,
              )}
              data-testid="user-huddle-indicator"
              role="img"
            >
              🎧
            </span>
          </TooltipTrigger>
          <TooltipContent>In a huddle</TooltipContent>
        </Tooltip>
      ) : null}
      {status ? (
        <Tooltip disableHoverableContent>
          <TooltipTrigger asChild>
            <span
              aria-label={`${statusEmoji} ${status.text || "User status"}`}
              className={cn(
                "inline-flex cursor-default items-center justify-center leading-none",
                indicatorTextClass,
              )}
              data-testid="user-status-indicator"
              role="img"
            >
              <StatusEmoji
                className={statusEmojiClass}
                decorative
                value={statusEmoji}
              />
            </span>
          </TooltipTrigger>
          <TooltipContent>{status.text || "Status set"}</TooltipContent>
        </Tooltip>
      ) : null}
    </span>
  );
}
