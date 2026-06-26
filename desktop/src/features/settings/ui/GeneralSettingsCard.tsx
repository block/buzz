import { Switch } from "@/shared/ui/switch";
import { useCloseToTray } from "../hooks/useCloseToTray";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

export function GeneralSettingsCard() {
  const { enabled, setEnabled } = useCloseToTray();

  return (
    <section className="min-w-0" data-testid="settings-general">
      <SettingsSectionHeader
        title="General"
        description="App behavior on this machine."
      />

      <SettingsOptionGroup>
        <SettingsOptionRow>
          <div className="min-w-0">
            <label
              className="text-sm font-medium"
              htmlFor="close-to-tray-switch"
            >
              Keep Buzz running in the tray when closed
            </label>
            <p className="text-sm font-normal text-muted-foreground">
              Closing the window keeps Buzz in the tray instead of quitting.
              Reopen or quit from the tray icon.
            </p>
          </div>
          <Switch
            checked={enabled}
            data-testid="close-to-tray-toggle"
            id="close-to-tray-switch"
            onCheckedChange={setEnabled}
          />
        </SettingsOptionRow>
      </SettingsOptionGroup>
    </section>
  );
}
