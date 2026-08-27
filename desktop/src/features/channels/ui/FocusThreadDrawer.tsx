import type * as React from "react";

import { CoverDrawer } from "@/features/channels/ui/CoverDrawer";

type FocusThreadDrawerProps = {
  channelName: string;
  children: React.ReactNode;
  /**
   * Whether the thread has an edit in progress, which Escape must cancel before
   * it can dismiss the drawer. See `CoverDrawer`'s `escapeYieldsToContent`.
   */
  hasActiveEdit?: boolean;
  /** Accessible name for the drawer. Channel threads leave the default. */
  label?: string;
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
 *
 * The idle auxiliary panel reuses this presentation with its own `label`, which
 * is why the label is a prop rather than the constant the thread wants.
 */
export function FocusThreadDrawer({
  channelName,
  children,
  hasActiveEdit = false,
  label = "Thread",
  onClose,
}: FocusThreadDrawerProps) {
  return (
    <CoverDrawer
      ariaLabel={label}
      escapeYieldsToContent={hasActiveEdit}
      onClose={onClose}
      scrimLabel={`Back to #${channelName}`}
      testId="focus-thread-drawer"
    >
      {children}
    </CoverDrawer>
  );
}
