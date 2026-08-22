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

type LayoutScrollTarget = {
  messageId: string;
  threadHeadId: string;
};

export function getScopedLayoutScrollTargetId({
  activeThreadHeadId,
  layoutTarget,
}: {
  activeThreadHeadId: string | null;
  layoutTarget: LayoutScrollTarget | null;
}): string | null {
  return layoutTarget?.threadHeadId === activeThreadHeadId
    ? layoutTarget.messageId
    : null;
}

/** Scroll region whose top-visible row is preserved across a mode switch. */
export const THREAD_BODY_SELECTOR = '[data-testid="message-thread-body"]';

type ThreadViewModeSwitchOptions = {
  /**
   * Identity of the surface the captured anchor belongs to.
   *
   * A row id only means something inside the surface it was read from, so the
   * anchor is dropped when this changes. Threads pass the open thread head;
   * surfaces that do not preserve a reading position pass `null`.
   */
  activeThreadHeadId: string | null;
  /**
   * Scroll region to read the anchor from, and to focus when the switch was not
   * keyboard-driven. Defaults to the channel thread body.
   */
  bodySelector?: string;
  externalScrollTargetId?: string | null;
  onExternalTargetResolved?: () => void;
  onModeChange?: (mode: ThreadViewMode) => void;
  /**
   * Whether the reader's row is restored after the switch.
   *
   * Threads restore it. The agent work viewer deliberately does not: its
   * transcript rows use `content-visibility: auto` with an over-reserved
   * `contain-intrinsic-size`, so the scroll height shrinks as rows paint after
   * the layout change and a restored position slides out from under the reader.
   * Snapping to the latest activity is the honest behaviour until transcript
   * rows reserve their real heights.
   */
  preserveReadingPosition?: boolean;
};

/** Preserves the row being read while a surface changes presentation. */
export function useThreadViewModeSwitch({
  activeThreadHeadId,
  bodySelector = THREAD_BODY_SELECTOR,
  externalScrollTargetId = null,
  onExternalTargetResolved,
  onModeChange,
  preserveReadingPosition = true,
}: ThreadViewModeSwitchOptions) {
  const [layoutScrollTarget, setLayoutScrollTarget] =
    React.useState<LayoutScrollTarget | null>(null);
  const layoutScrollTargetId = getScopedLayoutScrollTargetId({
    activeThreadHeadId,
    layoutTarget: layoutScrollTarget,
  });

  React.useEffect(() => {
    setLayoutScrollTarget((current) =>
      current?.threadHeadId === activeThreadHeadId ? current : null,
    );
  }, [activeThreadHeadId]);

  const changeThreadViewMode = React.useCallback(
    (mode: ThreadViewMode, restoreFocus: boolean) => {
      const body = document.querySelector<HTMLElement>(bodySelector);
      const anchorId = preserveReadingPosition
        ? findTopVisibleThreadMessageId(body)
        : null;

      setLayoutScrollTarget(
        anchorId && activeThreadHeadId
          ? { messageId: anchorId, threadHeadId: activeThreadHeadId }
          : null,
      );
      onModeChange?.(mode);
      setThreadViewMode(mode);
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          document
            .querySelector<HTMLElement>(
              restoreFocus
                ? '[data-testid="thread-view-mode-toggle"]'
                : bodySelector,
            )
            ?.focus({ preventScroll: true });
        });
      });
    },
    [activeThreadHeadId, bodySelector, onModeChange, preserveReadingPosition],
  );

  const resolveScrollTarget = React.useCallback(
    (settledMessageId?: string) => {
      const resolution = getResolvedThreadTargets({
        externalTargetId: externalScrollTargetId,
        layoutTargetId: layoutScrollTargetId,
      });
      if (resolution.resolveExternal) onExternalTargetResolved?.();
      if (settledMessageId) {
        setLayoutScrollTarget((current) =>
          current?.threadHeadId === activeThreadHeadId &&
          current.messageId === settledMessageId
            ? null
            : current,
        );
      }
    },
    [
      activeThreadHeadId,
      externalScrollTargetId,
      layoutScrollTargetId,
      onExternalTargetResolved,
    ],
  );

  return {
    changeThreadViewMode,
    layoutScrollTargetId,
    resolveScrollTarget,
  };
}
