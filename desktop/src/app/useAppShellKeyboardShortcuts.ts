import * as React from "react";

import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";

type AppShellKeyboardShortcutsOptions = {
  canSearchCurrentChannel: boolean;
  disabled: boolean;
  onBrowseChannels: () => void;
  onCreateChannel: () => void;
  onGoHome: () => unknown;
  onNewMessage: () => unknown;
  onSearchCurrentChannel: () => void;
  onSearchEverything: () => void;
};

export function useAppShellKeyboardShortcuts({
  canSearchCurrentChannel,
  disabled,
  onBrowseChannels,
  onCreateChannel,
  onGoHome,
  onNewMessage,
  onSearchCurrentChannel,
  onSearchEverything,
}: AppShellKeyboardShortcutsOptions) {
  React.useLayoutEffect(() => {
    if (disabled) return;

    function handleKeyDown(event: KeyboardEvent) {
      if (
        !hasPrimaryShortcutModifier(event) ||
        event.altKey ||
        event.repeat ||
        event.defaultPrevented
      ) {
        return;
      }

      const key = event.key.toLowerCase();
      if (key === "f" && !event.shiftKey && canSearchCurrentChannel) {
        event.preventDefault();
        onSearchCurrentChannel();
        return;
      }

      if (key === "k" && !event.shiftKey) {
        event.preventDefault();
        onSearchEverything();
        return;
      }

      if (key === "k" && event.shiftKey) {
        event.preventDefault();
        void onNewMessage();
        return;
      }

      if (key === "n" && event.shiftKey) {
        event.preventDefault();
        onCreateChannel();
        return;
      }

      if (key === "o" && event.shiftKey) {
        event.preventDefault();
        onBrowseChannels();
        return;
      }

      if (key === "a" && event.shiftKey) {
        event.preventDefault();
        void onGoHome();
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [
    canSearchCurrentChannel,
    disabled,
    onBrowseChannels,
    onCreateChannel,
    onGoHome,
    onNewMessage,
    onSearchCurrentChannel,
    onSearchEverything,
  ]);
}
