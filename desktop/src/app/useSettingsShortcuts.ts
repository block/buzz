import * as React from "react";

import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";

type UseSettingsShortcutsOptions = {
  onOpenSettings: () => void;
  open?: boolean;
};

export function useSettingsShortcuts({
  onOpenSettings,
  open,
}: UseSettingsShortcutsOptions) {
  React.useLayoutEffect(() => {
    if (open === undefined) return;

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
      event.stopImmediatePropagation();
      onOpenSettings();
    }

    window.addEventListener("keydown", handleKeyDown, true);
    return () => {
      window.removeEventListener("keydown", handleKeyDown, true);
    };
  }, [onOpenSettings, open]);
}
