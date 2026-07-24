import * as React from "react";

import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";

type UseGlobalShortcutsOptions = {
  /** Whether global navigation shortcuts are temporarily disabled. */
  disabled: boolean;
  /** ⌘K — open the search / quick-switch dialog. */
  onOpenSearch: () => void;
  /** ⇧⌘K — open the new direct-message composer. */
  onOpenNewDm: () => void;
  /** ⇧⌘N — open the create-channel dialog. */
  onOpenCreateChannel: () => void;
  /** ⇧⌘O — open the channel browser. */
  onBrowseChannels: () => void;
  /** ⇧⌘A — navigate to the home feed. */
  onGoHome: () => void;
};

/**
 * Window-level global navigation shortcuts. Dormant while `disabled` (e.g. when
 * settings is open). Respects `event.defaultPrevented` so a focused surface —
 * e.g. the composer consuming ⌘K for its link editor — can claim the key first
 * via an element-level handler that runs before this bubble listener.
 */
export function useGlobalShortcuts({
  disabled,
  onOpenSearch,
  onOpenNewDm,
  onOpenCreateChannel,
  onBrowseChannels,
  onGoHome,
}: UseGlobalShortcutsOptions): void {
  React.useLayoutEffect(() => {
    if (disabled) {
      return;
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (!hasPrimaryShortcutModifier(event) || event.altKey || event.repeat) {
        return;
      }
      if (event.defaultPrevented) {
        return;
      }

      const key = event.key.toLowerCase();
      if (key === "k" && !event.shiftKey) {
        event.preventDefault();
        onOpenSearch();
      } else if (key === "k" && event.shiftKey) {
        event.preventDefault();
        onOpenNewDm();
      } else if (key === "n" && event.shiftKey) {
        event.preventDefault();
        onOpenCreateChannel();
      } else if (key === "o" && event.shiftKey) {
        event.preventDefault();
        onBrowseChannels();
      } else if (key === "a" && event.shiftKey) {
        event.preventDefault();
        onGoHome();
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [
    disabled,
    onBrowseChannels,
    onGoHome,
    onOpenCreateChannel,
    onOpenNewDm,
    onOpenSearch,
  ]);
}
