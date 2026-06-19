import * as React from "react";

import type { GooseAppAvatarAsset } from "@/shared/avatars/gooseAppAvatars";
import { cn } from "@/shared/lib/cn";

type GooseAppAvatarMediaProps = {
  asset: GooseAppAvatarAsset;
  alt: string;
  animateOnHover?: boolean;
  className?: string;
  mediaClassName?: string;
  onError?: () => void;
  playVideo?: boolean;
  testId?: string;
};

export function GooseAppAvatarMedia({
  asset,
  alt,
  animateOnHover = true,
  className,
  mediaClassName,
  onError,
  playVideo = false,
  testId,
}: GooseAppAvatarMediaProps) {
  const [isHovered, setIsHovered] = React.useState(false);
  const canAnimate = Boolean(asset.webmUrl || asset.hevcUrl);
  const showVideo = canAnimate && (playVideo || (animateOnHover && isHovered));

  return (
    <span
      aria-label={alt}
      className={cn("block h-full w-full", className)}
      onMouseEnter={animateOnHover ? () => setIsHovered(true) : undefined}
      onMouseLeave={animateOnHover ? () => setIsHovered(false) : undefined}
      role="img"
    >
      {showVideo ? (
        <video
          autoPlay
          className={cn("h-full w-full object-contain", mediaClassName)}
          data-testid={testId}
          loop
          muted
          onError={onError}
          playsInline
          poster={asset.posterUrl ?? undefined}
          preload="metadata"
        >
          {asset.hevcUrl ? (
            <source src={asset.hevcUrl} type='video/mp4; codecs="hvc1"' />
          ) : null}
          {asset.webmUrl ? (
            <source src={asset.webmUrl} type="video/webm" />
          ) : null}
        </video>
      ) : asset.posterUrl ? (
        <img
          alt=""
          className={cn("h-full w-full object-contain", mediaClassName)}
          data-testid={testId}
          onError={onError}
          referrerPolicy="no-referrer"
          src={asset.posterUrl}
        />
      ) : null}
    </span>
  );
}
