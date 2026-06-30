import * as React from "react";
import type { ChannelSurfaceTab } from "./ChannelScreenHeader";

type RouteSnapshot = {
  activeChannelId: string | null;
  openThreadHeadId: string | null;
  targetMessageId: string | null;
};

type UseResetChannelSurfaceTabOnRouteOpenOptions = RouteSnapshot & {
  setActiveSurfaceTab: React.Dispatch<React.SetStateAction<ChannelSurfaceTab>>;
};

export function useResetChannelSurfaceTabOnRouteOpen({
  activeChannelId,
  openThreadHeadId,
  setActiveSurfaceTab,
  targetMessageId,
}: UseResetChannelSurfaceTabOnRouteOpenOptions) {
  const previousRouteRef = React.useRef<RouteSnapshot>({
    activeChannelId,
    openThreadHeadId,
    targetMessageId,
  });

  React.useEffect(() => {
    const previous = previousRouteRef.current;
    const shouldReset =
      previous.activeChannelId !== activeChannelId ||
      (openThreadHeadId !== null &&
        previous.openThreadHeadId !== openThreadHeadId) ||
      (targetMessageId !== null &&
        previous.targetMessageId !== targetMessageId);

    previousRouteRef.current = {
      activeChannelId,
      openThreadHeadId,
      targetMessageId,
    };

    if (shouldReset) {
      setActiveSurfaceTab("messages");
    }
  }, [activeChannelId, openThreadHeadId, setActiveSurfaceTab, targetMessageId]);
}
