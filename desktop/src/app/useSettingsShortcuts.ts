import * as React from "react";
import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { useUpdaterContext } from "@/features/settings/hooks/UpdaterProvider";
import type { SettingsSection } from "@/features/settings/ui/SettingsPanels";
import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";

type UseSettingsShortcutsOptions = {
  onClose: () => void;
  onOpenSettings: (section?: SettingsSection) => void;
  open?: boolean;
};

export function useSettingsShortcuts({
  onClose,
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
      if (open) {
        onClose();
        return;
      }

      onOpenSettings();
    }

    window.addEventListener("keydown", handleKeyDown, true);
    return () => {
      window.removeEventListener("keydown", handleKeyDown, true);
    };
  }, [onClose, onOpenSettings, open]);

  // On macOS the native app menu owns these actions: its Settings… item
  // claims the ⌘, key equivalent (resolved before the webview sees any key
  // event, so the keydown path above never fires there), and Check for
  // Updates… has no shortcut at all. Both arrive as Tauri events instead.
  const { checkForUpdate } = useUpdaterContext();
  const handleMenuOpenSettings = React.useEffectEvent(() => {
    if (open) {
      onClose();
      return;
    }
    onOpenSettings();
  });
  // Land on the updates section with the check already running, mirroring
  // the section's own button.
  const handleMenuCheckForUpdates = React.useEffectEvent(() => {
    onOpenSettings("updates");
    void checkForUpdate();
  });
  const enabled = open !== undefined;
  React.useEffect(() => {
    if (!enabled || !isTauri()) {
      return;
    }

    const unlistenSettings = listen("menu-open-settings", () => {
      handleMenuOpenSettings();
    });
    const unlistenUpdates = listen("menu-check-for-updates", () => {
      handleMenuCheckForUpdates();
    });

    return () => {
      void unlistenSettings.then((unlisten) => unlisten());
      void unlistenUpdates.then((unlisten) => unlisten());
    };
  }, [enabled]);
}
