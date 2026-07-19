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

export function getResolvedThreadTargets({
  externalTargetId,
  layoutTargetId,
}: {
  externalTargetId: string | null;
  layoutTargetId: string | null;
}) {
  return {
    resolveExternal:
      layoutTargetId === null || layoutTargetId === externalTargetId,
    resolveLayout: layoutTargetId !== null,
  };
}

type ThreadViewModeSwitchOptions = {
  externalScrollTargetId: string | null;
  onExternalTargetResolved: () => void;
  onModeChange?: (mode: ThreadViewMode) => void;
};

/** Preserves the reply being read while the thread changes presentation. */
export function useThreadViewModeSwitch({
  externalScrollTargetId,
  onExternalTargetResolved,
  onModeChange,
}: ThreadViewModeSwitchOptions) {
  const [layoutScrollTargetId, setLayoutScrollTargetId] = React.useState<
    string | null
  >(null);

  const changeThreadViewMode = React.useCallback(
    (mode: ThreadViewMode) => {
      const body = document.querySelector<HTMLElement>(
        '[data-testid="message-thread-body"]',
      );
      const anchorId = findTopVisibleThreadMessageId(body);

      setLayoutScrollTargetId(anchorId);
      onModeChange?.(mode);
      setThreadViewMode(mode);
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          document
            .querySelector<HTMLElement>(
              '[data-testid="thread-view-mode-toggle"]',
            )
            ?.focus({ preventScroll: true });
        });
      });
    },
    [onModeChange],
  );

  const resolveScrollTarget = React.useCallback(() => {
    const resolution = getResolvedThreadTargets({
      externalTargetId: externalScrollTargetId,
      layoutTargetId: layoutScrollTargetId,
    });
    if (resolution.resolveLayout) setLayoutScrollTargetId(null);
    if (resolution.resolveExternal) onExternalTargetResolved();
  }, [externalScrollTargetId, layoutScrollTargetId, onExternalTargetResolved]);

  return {
    changeThreadViewMode,
    layoutScrollTargetId,
    resolveScrollTarget,
  };
}
