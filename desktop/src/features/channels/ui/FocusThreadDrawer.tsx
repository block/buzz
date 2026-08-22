import type * as React from "react";

import { CoverDrawer } from "@/features/channels/ui/CoverDrawer";

type FocusThreadDrawerProps = {
  channelName: string;
  children: React.ReactNode;
  onClose: () => void;
};

/**
 * The focus-mode thread presentation: a {@link CoverDrawer} holding the thread.
 *
 * Everything about the surface itself — motion, scrim, Escape, focus
 * capture/restore — lives in `CoverDrawer`. Switching this thread to the split
 * pane is not a dismissal and must not restore focus to whatever opened the
 * thread; that case is handled where the switch happens, by releasing the
 * drawer's focus slot before this unmounts. See `useThreadViewModeSwitch`.
 */
export function FocusThreadDrawer({
  channelName,
  children,
  onClose,
}: FocusThreadDrawerProps) {
  return (
    <CoverDrawer
      ariaLabel="Thread"
      onClose={onClose}
      scrimLabel={`Back to #${channelName}`}
      testId="focus-thread-drawer"
    >
      {children}
    </CoverDrawer>
  );
}
