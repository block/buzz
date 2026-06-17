import * as React from "react";

import { parseAnimatedAvatarUrl } from "@/shared/lib/animatedAvatar";
import { cn } from "@/shared/lib/cn";
import { getInitials } from "@/shared/lib/initials";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { Avatar, AvatarFallback, AvatarImage } from "@/shared/ui/avatar";

type UserAvatarSize = "xs" | "sm" | "md";

const sizeClasses: Record<UserAvatarSize, string> = {
  xs: "h-5 w-5 text-3xs",
  sm: "h-6 w-6 text-2xs",
  md: "h-10 w-10 text-xs",
};

type UserAvatarProps = {
  avatarUrl: string | null;
  displayName: string;
  size?: UserAvatarSize;
  accent?: boolean;
  className?: string;
  testId?: string;
};

export function UserAvatar({
  avatarUrl,
  displayName,
  size = "md",
  accent = false,
  className,
  testId,
}: UserAvatarProps) {
  const initials = getInitials(displayName);
  // Animated avatars show their static poster frame until hovered, then play
  // the animation.
  const animated = parseAnimatedAvatarUrl(avatarUrl);
  const [isHovered, setIsHovered] = React.useState(false);
  const src = animated
    ? rewriteRelayUrl(isHovered ? animated.animationUrl : animated.posterUrl)
    : avatarUrl
      ? rewriteRelayUrl(avatarUrl)
      : null;

  return (
    <Avatar
      // Animated avatars carry their own backdrop disc and transparent
      // surroundings — any container fill would flatten the pop-out.
      className={cn(sizeClasses[size], !animated && "shadow-xs", className)}
      onMouseEnter={animated ? () => setIsHovered(true) : undefined}
      onMouseLeave={animated ? () => setIsHovered(false) : undefined}
    >
      {src ? (
        <AvatarImage
          alt={`${displayName} avatar`}
          className={cn("object-cover", !animated && "bg-secondary")}
          data-testid={testId ? `${testId}-image` : undefined}
          referrerPolicy="no-referrer"
          src={src}
        />
      ) : null}
      <AvatarFallback
        className={cn(
          "font-semibold",
          accent
            ? "bg-primary text-primary-foreground"
            : "bg-secondary text-secondary-foreground",
        )}
        data-testid={testId ? `${testId}-fallback` : undefined}
        delayMs={200}
      >
        {initials}
      </AvatarFallback>
    </Avatar>
  );
}
