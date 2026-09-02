import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

export function ProjectHomeChannelRedirect({
  channelId,
}: {
  channelId: string;
}) {
  const { goChannel, goHome } = useAppNavigation();

  React.useEffect(() => {
    if (channelId) {
      void goChannel(channelId, { replace: true });
    } else {
      void goHome({ replace: true });
    }
  }, [channelId, goChannel, goHome]);

  return <ViewLoadingFallback includeHeader kind="channel" />;
}
