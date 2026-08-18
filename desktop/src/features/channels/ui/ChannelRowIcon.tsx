import { FileText, Hash, Lock } from "lucide-react";

import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

export type ChannelRowIconKind = "hash" | "lock" | "forum";

/**
 * Pure discriminator used by `<ChannelRowIcon />` so the icon choice is
 * unit-testable without rendering React. Channels whose visibility is
 * `private` always show a lock, regardless of type — the same precedence
 * the sidebar applies, so the two surfaces stay in sync.
 */
export function getChannelRowIconKind(channel: Channel): ChannelRowIconKind {
  if (channel.visibility === "private") return "lock";
  if (channel.channelType === "forum") return "forum";
  return "hash";
}

/**
 * Render the affordance that names a channel the same way the sidebar does:
 * a lock for private channels, a file icon for forums, and a hash for
 * everything else. Extracted so the channel browser and sidebar can't drift
 * on which icon they show for which channel kind.
 */
export function ChannelRowIcon({
  channel,
  className,
}: {
  channel: Channel;
  className?: string;
}) {
  switch (getChannelRowIconKind(channel)) {
    case "lock":
      return <Lock className={cn("h-4 w-4", className)} />;
    case "forum":
      return <FileText className={cn("h-4 w-4", className)} />;
    case "hash":
      return <Hash className={cn("h-4 w-4", className)} />;
  }
}