import { FileText, Hash, Lock } from "lucide-react";

import { useIsProjectHomeChannel } from "@/features/projects/lib/projectHomeChannel";
import { ProjectChannelIcon } from "@/features/projects/ui/ProjectChannelIcon";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

/** Stream/forum glyph for a channel, using the project mark on project homes. */
export function ChannelGlyph({
  channel,
  className,
}: {
  channel: Pick<Channel, "channelType" | "id" | "visibility">;
  className?: string;
}) {
  const projectHome = useIsProjectHomeChannel(channel.id);
  const iconClass = cn("size-4 shrink-0", className);

  if (projectHome) {
    return <ProjectChannelIcon className={iconClass} />;
  }
  if (channel.visibility === "private") {
    return <Lock className={iconClass} />;
  }
  if (channel.channelType === "forum") {
    return <FileText className={iconClass} />;
  }
  return <Hash className={iconClass} />;
}
