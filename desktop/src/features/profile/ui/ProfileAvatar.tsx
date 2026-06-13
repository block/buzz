import * as React from "react";
import { UserRound } from "lucide-react";

import { parseAnimatedAvatarUrl } from "@/shared/lib/animatedAvatar";
import { cn } from "@/shared/lib/cn";
import { getInitials } from "@/shared/lib/initials";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { Avatar, AvatarFallback, AvatarImage } from "@/shared/ui/avatar";
import { Spinner } from "@/shared/ui/spinner";

type ProfileAvatarProps = {
  avatarUrl: string | null;
  avatarDataUrl?: string | null;
  label: string;
  className?: string;
  iconClassName?: string;
  plain?: boolean;
  showAnimationLoader?: boolean;
  testId?: string;
};

export function ProfileAvatar({
  avatarUrl,
  avatarDataUrl,
  label,
  className,
  iconClassName,
  plain = false,
  showAnimationLoader = false,
  testId,
}: ProfileAvatarProps) {
  const initials = getInitials(label);

  // Animated avatars show their static poster frame until hovered, then play
  // the animation.
  const animated = parseAnimatedAvatarUrl(avatarUrl);
  const [isHovered, setIsHovered] = React.useState(false);
  const [loadedAnimationSrc, setLoadedAnimationSrc] = React.useState<
    string | null
  >(null);
  const baseUrl = animated
    ? isHovered
      ? animated.animationUrl
      : animated.posterUrl
    : avatarUrl;

  // Compute the live (proxied) source. Failures are tracked per resolved URL so
  // the poster and hover animation can recover independently.
  const liveSrc = baseUrl ? rewriteRelayUrl(baseUrl) : null;
  const animationSrc = animated ? rewriteRelayUrl(animated.animationUrl) : null;
  const posterSrc = animated ? rewriteRelayUrl(animated.posterUrl) : null;
  const [failedSrc, setFailedSrc] = React.useState<string | null>(null);
  const liveFailed = liveSrc !== null && failedSrc === liveSrc;
  const posterFailed = posterSrc !== null && failedSrc === posterSrc;

  // When the relay is unreachable the proxied avatar URL 404s/times out; fall
  // back to the locally cached data URL instead of dropping to initials.
  const src = liveFailed
    ? (avatarDataUrl ?? undefined)
    : (liveSrc ?? avatarDataUrl ?? undefined);
  const shouldShowFallback = src === undefined || (!animated && liveFailed);
  const shouldUseAnimationLoader = showAnimationLoader && animated !== null;
  const shouldShowAnimationLoader =
    shouldUseAnimationLoader &&
    isHovered &&
    animationSrc !== null &&
    liveSrc === animationSrc &&
    failedSrc !== animationSrc &&
    loadedAnimationSrc !== animationSrc;
  const posterUnderlaySrc = shouldShowAnimationLoader
    ? posterFailed
      ? (avatarDataUrl ?? undefined)
      : (posterSrc ?? avatarDataUrl ?? undefined)
    : undefined;

  return (
    <Avatar
      className={cn(
        "shrink-0 text-primary shadow-xs",
        // Animated avatars carry their own backdrop disc and transparent
        // surroundings — any container fill would flatten the pop-out.
        plain || animated ? "bg-transparent shadow-none" : "bg-primary/20",
        className,
      )}
      data-testid={testId}
      onMouseEnter={animated ? () => setIsHovered(true) : undefined}
      onMouseLeave={animated ? () => setIsHovered(false) : undefined}
    >
      {posterUnderlaySrc ? (
        <img
          alt=""
          aria-hidden="true"
          className="absolute inset-0 h-full w-full object-cover"
          draggable={false}
          referrerPolicy="no-referrer"
          src={posterUnderlaySrc}
        />
      ) : null}
      {src !== undefined ? (
        <AvatarImage
          alt={`${label} avatar`}
          className={cn(
            "object-cover",
            shouldUseAnimationLoader && "absolute inset-0",
          )}
          data-testid={testId ? `${testId}-image` : undefined}
          onLoadingStatusChange={(status) => {
            if (status === "error") setFailedSrc(liveSrc);
            if (status === "loaded" && src === liveSrc) {
              setFailedSrc(null);
              if (liveSrc === animationSrc) {
                setLoadedAnimationSrc(liveSrc);
              }
            }
          }}
          referrerPolicy="no-referrer"
          src={src}
        />
      ) : null}
      {shouldShowAnimationLoader ? (
        <span
          aria-live="polite"
          className="pointer-events-none absolute inset-0 grid place-items-center rounded-[inherit] bg-background/35 text-foreground/80"
          data-testid={testId ? `${testId}-animation-loader` : undefined}
        >
          <Spinner
            aria-label="Loading animated avatar"
            className="h-1/4 w-1/4 min-h-3 min-w-3 max-h-5 max-w-5"
          />
        </span>
      ) : null}
      {shouldShowFallback ? (
        <AvatarFallback
          className={cn(
            "font-semibold text-primary",
            plain || animated ? "bg-transparent" : "bg-primary/20",
          )}
          data-testid={testId ? `${testId}-fallback` : undefined}
          delayMs={src === undefined ? undefined : 200}
        >
          {initials.length > 0 ? (
            initials
          ) : (
            <UserRound className={iconClassName} />
          )}
        </AvatarFallback>
      ) : null}
    </Avatar>
  );
}
