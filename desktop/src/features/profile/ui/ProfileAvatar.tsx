import * as React from "react";
import { UserRound } from "lucide-react";

import { useAvatarPresentation } from "@/features/profile/avatarPresentationStore";
import { parseAnimatedAvatarUrl } from "@/shared/lib/animatedAvatar";
import { cn } from "@/shared/lib/cn";
import { getInitials } from "@/shared/lib/initials";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { Avatar, AvatarFallback, AvatarImage } from "@/shared/ui/avatar";
import { Spinner } from "@/shared/ui/spinner";

/**
 * A `data:` URL is inlined bytes — rendering it makes no network request, so an
 * untrusted publisher cannot use it to observe the viewer's IP or browse
 * timing. Every other scheme (`http(s):`, `blob:`, relative) can reach the
 * network and is suppressed under `untrusted`.
 */
function isInlineDataUrl(url: string): boolean {
  return /^data:/i.test(url);
}

type ProfileAvatarProps = {
  avatarUrl: string | null;
  avatarDataUrl?: string | null;
  label: string;
  className?: string;
  iconClassName?: string;
  imageClassName?: string;
  plain?: boolean;
  shape?: "circle" | "squircle";
  testId?: string;
  /**
   * Suppress every network image request for a publisher-controlled avatar
   * URL, rendering the initials/icon placeholder instead. Community-catalog
   * browse projects avatar URLs straight from untrusted publications; loading
   * them would hand the viewer's IP and browse timing to up to 64 attacker-
   * chosen hosts before the user adds anything. Inline `data:` avatars (e.g.
   * emoji avatars, and a trusted locally cached `avatarDataUrl`) carry no
   * network origin, so they still render — only network-capable schemes are
   * blocked.
   */
  untrusted?: boolean;
};

export function ProfileAvatar({
  avatarUrl,
  avatarDataUrl,
  label,
  className,
  iconClassName,
  imageClassName,
  plain = false,
  shape = "circle",
  testId,
  untrusted = false,
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
  // the poster and hover animation can recover independently. Under `untrusted`
  // (publisher-controlled catalog browse) only an inline `data:` URL renders —
  // it carries no network origin, so it can't leak the viewer's IP; every
  // network-capable scheme is suppressed to the placeholder. This keeps emoji
  // avatars (persisted as inline `data:image/svg+xml`) visible while blocking
  // the up-to-64 attacker-chosen host fetches Carl flagged.
  const liveSrc = !baseUrl
    ? null
    : untrusted
      ? isInlineDataUrl(baseUrl)
        ? baseUrl
        : null
      : rewriteRelayUrl(baseUrl);
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
        shape === "squircle" && "rounded-[30%]",
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
          className={cn(
            "object-cover",
            presentation?.state === "pending" && "brightness-75",
            imageClassName,
          )}
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
          className="pointer-events-none absolute inset-0 flex items-center justify-center text-white drop-shadow-sm"
          data-testid={testId ? `${testId}-upload-pending` : undefined}
          role="status"
        >
          <span className="flex size-7 items-center justify-center rounded-full bg-black/35">
            <Spinner aria-hidden="true" className="border-2" size={16} />
          </span>
        </span>
      ) : null}
    </Avatar>
  );
}
