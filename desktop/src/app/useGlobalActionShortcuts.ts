import * as React from "react";

import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";

/**
 * Global app-action shortcuts on the primary modifier: ⌘K quick search,
 * ⇧⌘K new DM, ⇧⌘N new channel, ⇧⌘O browse channels, ⇧⌘A home (Ctrl on
 * Windows/Linux). Suspended while `enabled` is false (settings open), which
 * mirrors the previous inline AppShell effect this was extracted from.
 */
export function useGlobalActionShortcuts({
  enabled,
  onGoHome,
  onOpenBrowseChannels,
  onOpenCreateChannel,
  onOpenNewDm,
  onOpenSearch,
}: {
  enabled: boolean;
  onGoHome: () => void;
  onOpenBrowseChannels: () => void;
  onOpenCreateChannel: () => void;
  onOpenNewDm: () => void;
  onOpenSearch: () => void;
}) {
  React.useLayoutEffect(() => {
    if (!enabled) {
      return;
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (!hasPrimaryShortcutModifier(event) || event.altKey || event.repeat) {
        return;
      }

      // A focused surface may claim the shortcut first — e.g. the composer
      // consumes ⌘K to open the link editor when text is selected. Its
      // element-level handler runs before this window-level bubble listener
      // and calls `preventDefault()`; respect that instead of also opening
      // the global dialog.
      if (event.defaultPrevented) {
        return;
      }

      const key = event.key.toLowerCase();
      if (key === "k" && !event.shiftKey) {
        event.preventDefault();
        onOpenSearch();
        return;
      }

      if (key === "k" && event.shiftKey) {
        event.preventDefault();
        onOpenNewDm();
        return;
      }

      if (key === "n" && event.shiftKey) {
        event.preventDefault();
        onOpenCreateChannel();
        return;
      }

      if (key === "o" && event.shiftKey) {
        event.preventDefault();
        onOpenBrowseChannels();
        return;
      }

      if (key === "a" && event.shiftKey) {
        event.preventDefault();
        onGoHome();
        return;
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [
    enabled,
    onGoHome,
    onOpenBrowseChannels,
    onOpenCreateChannel,
    onOpenNewDm,
    onOpenSearch,
  ]);
}
