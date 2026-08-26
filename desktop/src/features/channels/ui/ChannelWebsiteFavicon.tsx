import { Globe } from "lucide-react";
import * as React from "react";

import { channelWebsiteFaviconUrl } from "@/features/channels/lib/channelWebsites";
import { cn } from "@/shared/lib/cn";

type ChannelWebsiteFaviconProps = {
  className?: string;
  label: string;
  url: string;
};

export function ChannelWebsiteFavicon({
  className,
  label,
  url,
}: ChannelWebsiteFaviconProps) {
  const src = channelWebsiteFaviconUrl(url);
  const [failedSrc, setFailedSrc] = React.useState<string | null>(null);
  const failed = src !== null && failedSrc === src;

  if (!src || failed) {
    return <Globe aria-hidden className={cn("h-3.5 w-3.5", className)} />;
  }

  return (
    <img
      alt=""
      className={cn("h-3.5 w-3.5 rounded-sm", className)}
      data-testid="channel-website-favicon"
      onError={() => setFailedSrc(src)}
      src={src}
      title={label}
    />
  );
}
