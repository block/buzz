import { Switch } from "@/shared/ui/switch";
import { useCloseToTray } from "../hooks/useCloseToTray";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

export function GeneralSettingsCard() {
  const { enabled, isUpdating, setEnabled } = useCloseToTray();

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
              Keep Buzz running when the window closes
            </label>
            <p className="text-sm font-normal text-muted-foreground">
              Continue receiving notifications and running agents. Reopen Buzz
              from the Dock or system tray; use Quit to exit completely.
            </p>
          </div>
          <Switch
            checked={enabled}
            data-testid="close-to-tray-toggle"
            disabled={isUpdating}
            id="close-to-tray-switch"
            onCheckedChange={setEnabled}
          />
        </SettingsOptionRow>
      </SettingsOptionGroup>
    </section>
  );
}
