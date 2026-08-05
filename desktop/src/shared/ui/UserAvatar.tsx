import * as React from "react";

import { parseAnimatedAvatarUrl } from "@/shared/lib/animatedAvatar";
import {
  type AvatarSize,
  avatarSizeStyle,
  getAvatarSizeRem,
  getAvatarScale,
} from "@/shared/lib/avatarScale";
import { cn } from "@/shared/lib/cn";
import { getInitials } from "@/shared/lib/initials";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { useAvatarScale } from "@/shared/lib/useAvatarScale";
import { Avatar, AvatarFallback, AvatarImage } from "@/shared/ui/avatar";

const sizeTextClasses: Record<AvatarSize, string> = {
  xs: "text-3xs",
  sm: "text-2xs",
  md: "text-xs",
};

type UserAvatarProps = {
  avatarUrl: string | null;
  displayName: string;
  size?: AvatarSize;
  /**
   * When true (default), width/height follow Appearance → Avatar size.
   * Set false for decorative/editor surfaces that must ignore the slider.
   */
  appearanceScale?: boolean;
  /**
   * @deprecated Use {@link appearanceScale}. Kept temporarily for call-sites
   * still passing `messageScale`; treated as appearanceScale when provided.
   */
  messageScale?: boolean;
  accent?: boolean;
  className?: string;
  style?: React.CSSProperties;
  fallbackDelayMs?: number;
  testId?: string;
};

export function UserAvatar({
  avatarUrl,
  displayName,
  size = "md",
  appearanceScale,
  messageScale,
  accent = false,
  className,
  style,
  fallbackDelayMs = 200,
  testId,
}: UserAvatarProps) {
  // Default on; explicit messageScale remains supported as an alias.
  const scaleEnabled = appearanceScale ?? messageScale ?? true;
  const avatarScale = useAvatarScale();
  const initials = getInitials(displayName);
  const animated = parseAnimatedAvatarUrl(avatarUrl);
  const [isHovered, setIsHovered] = React.useState(false);
  const src = animated
    ? rewriteRelayUrl(isHovered ? animated.animationUrl : animated.posterUrl)
    : avatarUrl
      ? rewriteRelayUrl(avatarUrl)
      : null;

  const scaledStyle = scaleEnabled
    ? avatarSizeStyle(size, avatarScale)
    : undefined;
  const mergedStyle: React.CSSProperties | undefined = scaleEnabled
    ? { ...scaledStyle, ...style }
    : style;

  return (
    <Avatar
      className={cn(
        "shrink-0",
        sizeTextClasses[size],
        !animated && "shadow-xs",
        className,
      )}
      data-avatar-scale={scaleEnabled ? String(avatarScale) : undefined}
      onMouseEnter={animated ? () => setIsHovered(true) : undefined}
      onMouseLeave={animated ? () => setIsHovered(false) : undefined}
      style={mergedStyle}
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
        delayMs={fallbackDelayMs}
      >
        {initials}
      </AvatarFallback>
    </Avatar>
  );
}

/** @deprecated Prefer {@link getAvatarSizeRem} from `@/shared/lib/avatarScale`. */
export function getScaledMessageAvatarRem(
  size: AvatarSize = "md",
  scale = getAvatarScale(),
): number {
  return getAvatarSizeRem(size, scale);
}
