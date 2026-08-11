import { useEffect, useState } from "react";
import { ChevronDown } from "lucide-react";

import { isTauri } from "@tauri-apps/api/core";
import { Button } from "@/shared/ui/button";
import { isMacPlatform } from "@/shared/lib/platform";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";
import {
  CLOSE_TO_TRAY_DEFAULT,
  CLOSE_TO_TRAY_OPTIONS,
  type CloseToTrayBehavior,
  loadCloseToTrayBehavior,
  saveCloseToTrayBehavior,
} from "./closeToTrayLogic";

function optionLabelFor(behavior: CloseToTrayBehavior): string {
  return (
    CLOSE_TO_TRAY_OPTIONS.find((option) => option.value === behavior)?.label ??
    behavior
  );
}

/**
 * macOS-only preference for what happens when the main window closes (#4024).
 *
 * The backend close handler is macOS-only, so this card renders nothing on
 * other platforms (and outside the Tauri desktop shell, where the commands do
 * not exist).
 */
export function CloseToTraySettingsCard() {
  const isDesktopMac = isTauri() && isMacPlatform();
  const [behavior, setBehavior] =
    useState<CloseToTrayBehavior>(CLOSE_TO_TRAY_DEFAULT);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!isDesktopMac) {
      return;
    }
    let cancelled = false;
    void loadCloseToTrayBehavior().then((loaded) => {
      if (!cancelled) {
        setBehavior(loaded);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [isDesktopMac]);

  if (!isDesktopMac) {
    return null;
  }

  const current = CLOSE_TO_TRAY_OPTIONS.find((o) => o.value === behavior);

  return (
    <section className="min-w-0" data-testid="settings-close-to-tray">
      <SettingsSectionHeader
        title="When you close the window"
        description="Choose what Buzz does after you close its main window on macOS."
      />

      <SettingsOptionGroup>
        <SettingsOptionRow>
          <div className="min-w-0">
            <label
              className="text-sm font-medium"
              htmlFor="close-to-tray-select"
            >
              Close behavior
            </label>
            <p className="text-sm font-normal text-muted-foreground">
              Keeping Buzz running in the background preserves active local
              agents; choose minimize or quit if your window switcher does not
              show hidden apps.
            </p>
            {current && (
              <p className="mt-2 text-sm text-muted-foreground/80">
                {current.description}
              </p>
            )}
          </div>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                className="ml-4 shrink-0"
                disabled={saving}
                id="close-to-tray-select"
                variant="outline"
                data-testid="close-to-tray-select"
              >
                {optionLabelFor(behavior)}
                <ChevronDown className="opacity-70" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuRadioGroup
                value={behavior}
                onValueChange={(value) => {
                  const next = value as CloseToTrayBehavior;
                  setBehavior(next); // optimistic
                  setSaving(true);
                  void saveCloseToTrayBehavior(next)
                    .catch(() => {
                      // Reload the persisted value if the write failed.
                      void loadCloseToTrayBehavior().then(setBehavior);
                    })
                    .finally(() => setSaving(false));
                }}
              >
                {CLOSE_TO_TRAY_OPTIONS.map((option) => (
                  <DropdownMenuRadioItem
                    key={option.value}
                    value={option.value}
                    data-testid={`close-to-tray-option-${option.value}`}
                  >
                    {option.label}
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </SettingsOptionRow>
      </SettingsOptionGroup>
    </section>
  );
}
