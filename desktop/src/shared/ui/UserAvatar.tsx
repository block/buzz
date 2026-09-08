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
  md: "h-9 w-9 text-xs",
};

const fallbackColorClasses = [
  "bg-blue-500 text-white",
  "bg-emerald-500 text-white",
  "bg-amber-400 text-amber-950",
  "bg-rose-500 text-white",
  "bg-cyan-400 text-cyan-950",
  "bg-violet-500 text-white",
  "bg-orange-500 text-white",
] as const;

function fallbackColorClass(displayName: string) {
  const hash = Array.from(displayName.trim().toLowerCase()).reduce(
    (value, character) => (value * 31 + (character.codePointAt(0) ?? 0)) >>> 0,
    0,
  );
  return fallbackColorClasses[hash % fallbackColorClasses.length];
}

type UserAvatarProps = {
  avatarUrl: string | null;
  displayName: string;
  size?: UserAvatarSize;
  accent?: boolean;
  shape?: "circle" | "squircle";
  className?: string;
  fallbackDelayMs?: number;
  imageDraggable?: boolean;
  testId?: string;
};

export function UserAvatar({
  avatarUrl,
  displayName,
  size = "md",
  accent = false,
  shape,
  className,
  fallbackDelayMs = 200,
  imageDraggable,
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
  const resolvedShape = shape ?? "circle";
  const radiusClass =
    resolvedShape === "squircle" ? "rounded-[30%]" : "rounded-full";

  return (
    <Avatar
      // Animated avatars carry their own backdrop disc and transparent
      // surroundings — any container fill would flatten the pop-out.
      className={cn(
        sizeClasses[size],
        radiusClass,
        !animated && "shadow-xs",
        className,
      )}
      data-testid={testId}
      onMouseEnter={animated ? () => setIsHovered(true) : undefined}
      onMouseLeave={animated ? () => setIsHovered(false) : undefined}
    >
      {src ? (
        <AvatarImage
          alt={`${displayName} avatar`}
          className={cn("object-cover", !animated && "bg-secondary")}
          data-testid={testId ? `${testId}-image` : undefined}
          draggable={imageDraggable}
          referrerPolicy="no-referrer"
          src={src}
        />
      ) : null}
      <AvatarFallback
        className={cn(
          "font-semibold",
          accent
            ? "bg-primary text-primary-foreground"
            : fallbackColorClass(displayName),
        )}
        data-testid={testId ? `${testId}-fallback` : undefined}
        delayMs={fallbackDelayMs}
      >
        {initials}
      </AvatarFallback>
    </Avatar>
  );
}
