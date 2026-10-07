import * as React from "react";

type HuddleThreadIsolationOptions = {
  closeThread: (threadId: string | null) => void;
  isHuddleTranscript: boolean;
  openThreadHeadId: string | null;
  optimisticOpenThreadHeadId: string | null | undefined;
};

export function useHuddleThreadIsolation({
  closeThread,
  isHuddleTranscript,
  openThreadHeadId,
  optimisticOpenThreadHeadId,
}: HuddleThreadIsolationOptions): string | null {
  React.useEffect(() => {
    if (!isHuddleTranscript || openThreadHeadId === null) return;
    closeThread(null);
  }, [closeThread, isHuddleTranscript, openThreadHeadId]);
  if (isHuddleTranscript) return null;
  return optimisticOpenThreadHeadId === undefined
    ? openThreadHeadId
    : optimisticOpenThreadHeadId;
}
