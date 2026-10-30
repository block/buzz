import * as React from "react";

import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";
import {
  HUDDLE_SHORTCUT_EVENT,
  type HuddleShortcutDetail,
  REACT_TO_LATEST_SHORTCUT_EVENT,
  type ReactToLatestShortcutDetail,
} from "@/shared/lib/keyboard-shortcuts";

type AppShellKeyboardShortcutsOptions = {
  activeChannelId: string | null;
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
  activeChannelId,
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

    // Capture phase so these shortcuts work even when a composer or control
    // has focus and stops keydown propagation. Only shortcuts with no more
    // specific handler may live here: capture-phase handling of the others
    // would steal them (e.g. the composer's ⌘K link dialog).
    function handleCaptureShortcuts(event: KeyboardEvent) {
      if (event.repeat || event.defaultPrevented || !activeChannelId) return;

      const isHuddleShortcut =
        event.ctrlKey &&
        event.shiftKey &&
        !event.metaKey &&
        !event.altKey &&
        event.code === "Space";
      if (isHuddleShortcut) {
        event.preventDefault();
        window.dispatchEvent(
          new CustomEvent<HuddleShortcutDetail>(HUDDLE_SHORTCUT_EVENT, {
            detail: { channelId: activeChannelId },
          }),
        );
        return;
      }

      // `code` rather than `key`: Shift+\ produces "|" on many layouts.
      const isReactToLatestShortcut =
        hasPrimaryShortcutModifier(event) &&
        event.shiftKey &&
        !event.altKey &&
        event.code === "Backslash";
      if (isReactToLatestShortcut) {
        event.preventDefault();
        window.dispatchEvent(
          new CustomEvent<ReactToLatestShortcutDetail>(
            REACT_TO_LATEST_SHORTCUT_EVENT,
            { detail: { channelId: activeChannelId } },
          ),
        );
      }
    }

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

    window.addEventListener("keydown", handleCaptureShortcuts, {
      capture: true,
    });
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleCaptureShortcuts, {
        capture: true,
      });
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [
    activeChannelId,
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
