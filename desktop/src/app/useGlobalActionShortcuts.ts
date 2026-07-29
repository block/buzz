import * as React from "react";

import { toggleDisplayStyle } from "@/features/dev-mode/lib/displayStylePreference";
import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";

/**
 * Window-level primary-modifier shortcuts: ⌘K search, ⇧⌘K new DM, ⇧⌘N create
 * channel, ⇧⌘O browse channels, ⇧⌘A home, ⇧⌘D display style. Disabled while
 * settings is open (settings has its own shortcut scope).
 */
export function useGlobalActionShortcuts({
  settingsOpen,
  onOpenSearch,
  onOpenNewDm,
  onOpenCreateChannel,
  onOpenBrowseChannels,
  onGoHome,
}: {
  settingsOpen: boolean;
  onOpenSearch: () => void;
  onOpenNewDm: () => void;
  onOpenCreateChannel: () => void;
  onOpenBrowseChannels: () => void;
  onGoHome: () => void;
}) {
  React.useLayoutEffect(() => {
    if (settingsOpen) {
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

      if (key === "d" && event.shiftKey) {
        event.preventDefault();
        toggleDisplayStyle();
        return;
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [
    settingsOpen,
    onOpenSearch,
    onOpenNewDm,
    onOpenCreateChannel,
    onOpenBrowseChannels,
    onGoHome,
  ]);
}
