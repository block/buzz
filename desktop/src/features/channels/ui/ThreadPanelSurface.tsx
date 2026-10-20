import * as React from "react";

import { FocusThreadDrawer } from "@/features/channels/ui/FocusThreadDrawer";
import { usePresenceCoverage } from "@/features/channels/ui/useFocusDrawerPresence";

type ThreadPanelSurfaceProps = {
  channelName: string;
  children: React.ReactNode;
  covered: boolean;
  hasActiveEdit: boolean;
  isFocusDrawer: boolean;
  onClose: () => void;
};

/** Keeps a thread mounted while controlling its focus-drawer presentation. */
export const ThreadPanelSurface = React.forwardRef<
  HTMLDivElement,
  ThreadPanelSurfaceProps
>(function ThreadPanelSurface(
  { channelName, children, covered, hasActiveEdit, isFocusDrawer, onClose },
  ref,
) {
  return (
    <div
      aria-hidden={covered ? true : undefined}
      className="contents"
      data-testid="thread-surface"
      inert={covered ? true : undefined}
      ref={ref}
    >
      {isFocusDrawer ? (
        <FocusThreadDrawer
          channelName={channelName}
          escapeEnabled={!covered}
          hasActiveEdit={hasActiveEdit}
          onClose={onClose}
        >
          {children}
        </FocusThreadDrawer>
      ) : (
        children
      )}
    </div>
  );
});

/** Supplies covered-thread lifecycle and focus ownership for an overlay drawer. */
export function useThreadPanelSurface(
  open: boolean,
  onExitComplete: () => void,
) {
  const ref = React.useRef<HTMLDivElement>(null);
  const coverage = usePresenceCoverage(open);
  const markExitComplete = React.useCallback(() => {
    coverage.markExitComplete();
    onExitComplete();
  }, [coverage.markExitComplete, onExitComplete]);
  const restoreFocusTarget = React.useCallback(
    () =>
      ref.current?.querySelector<HTMLElement>(
        '[data-testid="auxiliary-panel-close"]',
      ) ?? null,
    [],
  );
  return { ...coverage, markExitComplete, ref, restoreFocusTarget };
}
