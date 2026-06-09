import * as React from "react";

import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";

type UseSettingsShortcutsOptions = {
  onClose: () => void;
  onOpenSettings: () => void;
  open: boolean;
};

export function useSettingsShortcuts({
  onClose,
  onOpenSettings,
  open,
}: UseSettingsShortcutsOptions) {
  React.useLayoutEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      const isSettingsShortcut =
        hasPrimaryShortcutModifier(event) &&
        !event.altKey &&
        !event.shiftKey &&
        (event.key === "," || event.code === "Comma");

      if (!isSettingsShortcut) {
        return;
      }

      event.preventDefault();
      if (open) {
        onClose();
        return;
      }

      onOpenSettings();
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [onClose, onOpenSettings, open]);
}
