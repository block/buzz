import * as React from "react";
import { CircleAlert, UserRound } from "lucide-react";

import { useAvatarPresentation } from "@/features/profile/avatarPresentationStore";
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
  imageClassName?: string;
  plain?: boolean;
  testId?: string;
};

export function ProfileAvatar({
  avatarUrl,
  avatarDataUrl,
  label,
  className,
  iconClassName,
  imageClassName,
  plain = false,
  testId,
}: ProfileAvatarProps) {
  const initials = getInitials(label);
  const presentation = useAvatarPresentation(avatarUrl);
  const presentedAvatarUrl = presentation?.displayUrl ?? avatarUrl;

  // Animated avatars show their static poster frame until hovered, then play
  // the animation.
  const animated = parseAnimatedAvatarUrl(presentedAvatarUrl);
  const [isHovered, setIsHovered] = React.useState(false);
  const baseUrl = animated
    ? isHovered
      ? animated.animationUrl
      : animated.posterUrl
    : presentedAvatarUrl;

  // Compute the live (proxied) source. Failures are tracked per resolved URL so
  // the poster and hover animation can recover independently.
  const liveSrc = baseUrl ? rewriteRelayUrl(baseUrl) : null;
  const [failedSrc, setFailedSrc] = React.useState<string | null>(null);
  const liveFailed = liveSrc !== null && failedSrc === liveSrc;

  // When the relay is unreachable the proxied avatar URL 404s/times out; fall
  // back to the locally cached data URL instead of dropping to initials.
  const src = liveFailed
    ? (avatarDataUrl ?? undefined)
    : (liveSrc ?? avatarDataUrl ?? undefined);
  const shouldShowFallback = src === undefined || (!animated && liveFailed);

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
      {src !== undefined ? (
        <AvatarImage
          alt={`${label} avatar`}
          className={cn("object-cover", imageClassName)}
          data-testid={testId ? `${testId}-image` : undefined}
          onLoadingStatusChange={(status) => {
            if (status === "error") setFailedSrc(liveSrc);
            if (status === "loaded" && src === liveSrc) {
              setFailedSrc(null);
            }
          }}
          referrerPolicy="no-referrer"
          src={src}
        />
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
      {presentation?.state === "pending" ? (
        <span
          aria-label="Avatar upload pending"
          className="absolute bottom-0 right-0 flex h-[28%] max-h-9 min-h-4 w-[28%] max-w-9 min-w-4 items-center justify-center rounded-full bg-amber-300 text-amber-950 shadow-sm ring-2 ring-background"
          data-testid={testId ? `${testId}-upload-pending` : undefined}
          role="status"
        >
          <Spinner aria-hidden="true" className="h-[60%] w-[60%] border-2" />
        </span>
      ) : null}
      {presentation?.state === "failed" ? (
        <span
          aria-label="Avatar upload failed"
          className="absolute bottom-0 right-0 flex h-[28%] max-h-9 min-h-4 w-[28%] max-w-9 min-w-4 items-center justify-center rounded-full bg-destructive text-destructive-foreground shadow-sm ring-2 ring-background"
          data-testid={testId ? `${testId}-upload-failed` : undefined}
          role="status"
        >
          <CircleAlert aria-hidden="true" className="h-[70%] w-[70%]" />
        </span>
      ) : null}
    </Avatar>
  );
}
