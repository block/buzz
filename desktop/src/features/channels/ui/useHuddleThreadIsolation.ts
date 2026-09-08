import * as React from "react";

type HuddleThreadIsolationOptions = {
  closeThread: (threadId: string | null) => void;
  isHuddleTranscript: boolean;
  openThreadHeadId: string | null;
  optimisticOpenThreadHeadId: string | null | undefined;
  requireThreadEditResolutionRef: React.RefObject<() => boolean>;
};

export function resolveHuddleOpenThreadHeadId({
  isHuddleTranscript,
  openThreadHeadId,
  optimisticOpenThreadHeadId,
}: Pick<
  HuddleThreadIsolationOptions,
  "isHuddleTranscript" | "openThreadHeadId" | "optimisticOpenThreadHeadId"
>): string | null {
  if (isHuddleTranscript) return null;
  return optimisticOpenThreadHeadId === undefined
    ? openThreadHeadId
    : optimisticOpenThreadHeadId;
}

export function useHuddleThreadIsolation({
  closeThread,
  isHuddleTranscript,
  openThreadHeadId,
  optimisticOpenThreadHeadId,
  requireThreadEditResolutionRef,
}: HuddleThreadIsolationOptions): string | null {
  React.useEffect(() => {
    if (!isHuddleTranscript || openThreadHeadId === null) return;
    if (!requireThreadEditResolutionRef.current()) return;
    closeThread(null);
  }, [
    closeThread,
    isHuddleTranscript,
    openThreadHeadId,
    requireThreadEditResolutionRef,
  ]);
  return resolveHuddleOpenThreadHeadId({
    isHuddleTranscript,
    openThreadHeadId,
    optimisticOpenThreadHeadId,
  });
}
