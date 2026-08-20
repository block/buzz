import * as React from "react";

export default function useChannelTargetCallbacks(
  clearMessageRouteTarget: (options: { replace: true }) => void,
  setThreadScrollTargetId: React.Dispatch<React.SetStateAction<string | null>>,
) {
  const handleThreadScrollTargetResolved = React.useCallback(() => {
    setThreadScrollTargetId(null);
  }, [setThreadScrollTargetId]);
  const handleTargetReached = React.useCallback(() => {
    clearMessageRouteTarget({ replace: true });
  }, [clearMessageRouteTarget]);
  return { handleTargetReached, handleThreadScrollTargetResolved };
}
