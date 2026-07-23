import * as React from "react";
import type { ScrollTargetAlignment } from "@/features/messages/ui/anchoredScrollTarget";

type ThreadScrollTargetState = {
  alignment: ScrollTargetAlignment;
  id: string | null;
};

type LoadedThreadReply = {
  message: { id: string };
};

export function useThreadOpenScrollTarget(
  threadHeadId: string | null,
  firstUnreadReplyId: string | null,
  loadedReplies: readonly LoadedThreadReply[],
  isFetchingReplies: boolean,
) {
  const [scrollTarget, setScrollTarget] =
    React.useState<ThreadScrollTargetState>({ alignment: "center", id: null });
  const latchedForHeadRef = React.useRef<string | null>(null);
  const lastReplyId = loadedReplies.at(-1)?.message.id ?? null;
  const isReadyToLatch =
    !isFetchingReplies &&
    lastReplyId !== null &&
    (firstUnreadReplyId === null ||
      loadedReplies.some((entry) => entry.message.id === firstUnreadReplyId));

  // Every caller-provided target is an explicit navigation (route target,
  // branch expansion, sent reply, or layout-mode restore), so it remains
  // centered even when its id happens to equal the unread anchor.
  const setScrollTargetId = React.useCallback<
    React.Dispatch<React.SetStateAction<string | null>>
  >((nextTarget) => {
    setScrollTarget((current) => {
      const nextId =
        typeof nextTarget === "function" ? nextTarget(current.id) : nextTarget;
      if (current.id === nextId && current.alignment === "center") {
        return current;
      }
      return { alignment: "center", id: nextId };
    });
  }, []);

  React.useEffect(() => {
    if (!threadHeadId) {
      latchedForHeadRef.current = null;
      return;
    }
    if (latchedForHeadRef.current === threadHeadId) {
      return;
    }

    // Explicit route/deep-link targets take precedence over the unread anchor.
    if (scrollTarget.id !== null) {
      latchedForHeadRef.current = threadHeadId;
      return;
    }

    // Wait for the refreshed reply set and divider snapshot to agree on a real
    // rendered row. This prevents a transient unread id from being frozen as
    // an invisible boundary.
    if (!isReadyToLatch) {
      return;
    }

    latchedForHeadRef.current = threadHeadId;
    const nextId = firstUnreadReplyId ?? lastReplyId;
    if (nextId) {
      setScrollTarget({
        alignment: firstUnreadReplyId ? "top-with-divider" : "bottom",
        id: nextId,
      });
    }
  }, [
    firstUnreadReplyId,
    isReadyToLatch,
    lastReplyId,
    scrollTarget.id,
    threadHeadId,
  ]);

  const clearScrollTarget = React.useCallback(() => {
    setScrollTarget((current) =>
      current.id === null && current.alignment === "center"
        ? current
        : { alignment: "center", id: null },
    );
  }, []);

  return [scrollTarget, setScrollTargetId, clearScrollTarget] as const;
}
