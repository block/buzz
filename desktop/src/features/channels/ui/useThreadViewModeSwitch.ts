import * as React from "react";

import {
  setThreadViewMode,
  type ThreadViewMode,
} from "@/features/channels/lib/threadViewModePreference";

export function findTopVisibleThreadMessageId(
  body: HTMLElement | null,
): string | null {
  if (!body) return null;

  const bodyTop = body.getBoundingClientRect().top;
  const visibleReply = Array.from(
    body.querySelectorAll<HTMLElement>("[data-message-id]"),
  ).find((row) => row.getBoundingClientRect().bottom > bodyTop);
  return visibleReply?.dataset.messageId ?? null;
}

/** Preserves the reply being read while the thread changes presentation. */
export function useThreadViewModeSwitch(onExternalTargetResolved: () => void) {
  const [layoutScrollTargetId, setLayoutScrollTargetId] = React.useState<
    string | null
  >(null);

  const changeThreadViewMode = React.useCallback((mode: ThreadViewMode) => {
    const body = document.querySelector<HTMLElement>(
      '[data-testid="message-thread-body"]',
    );
    const anchorId = findTopVisibleThreadMessageId(body);

    setLayoutScrollTargetId(anchorId);
    setThreadViewMode(mode);
  }, []);

  const resolveScrollTarget = React.useCallback(() => {
    if (layoutScrollTargetId) {
      setLayoutScrollTargetId(null);
      return;
    }
    onExternalTargetResolved();
  }, [layoutScrollTargetId, onExternalTargetResolved]);

  return {
    changeThreadViewMode,
    layoutScrollTargetId,
    resolveScrollTarget,
  };
}
