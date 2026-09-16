import { Switch } from "@/shared/ui/switch";
import { SettingsOptionRow } from "@/features/settings/ui/SettingsOptionGroup";

import {
  setEmbedInMain,
  setShowWindowsSection,
  setStartFullscreen,
  usePopoutSettings,
} from "../lib/popoutSettings";

export function WindowsSettings() {
  const settings = usePopoutSettings();
  const embedEnabled = settings.showWindowsSection && settings.embedInMain;

  return (
    <>
      <SettingsOptionRow data-testid="windows-show-section-row">
        <div className="min-w-0">
          <label
            className="text-sm font-medium"
            htmlFor="windows-show-section-switch"
          >
            Show Windows section
          </label>
          <p
            className="text-sm font-normal text-muted-foreground/70"
            data-settings-subcopy
          >
            List pop-out and embedded windows in the left sidebar.
          </p>
        </div>
        <Switch
          checked={settings.showWindowsSection}
          data-testid="windows-show-section-toggle"
          id="windows-show-section-switch"
          onCheckedChange={setShowWindowsSection}
        />
      </SettingsOptionRow>

      <SettingsOptionRow data-testid="windows-start-fullscreen-row">
        <div className="min-w-0">
          <label
            className="text-sm font-medium"
            htmlFor="windows-start-fullscreen-switch"
          >
            New windows start fullscreen
          </label>
          <p
            className="text-sm font-normal text-muted-foreground/70"
            data-settings-subcopy
          >
            Open newly created OS pop-out windows in fullscreen.
          </p>
        </div>
        <Switch
          checked={settings.startFullscreen}
          data-testid="windows-start-fullscreen-toggle"
          id="windows-start-fullscreen-switch"
          onCheckedChange={setStartFullscreen}
        />
      </SettingsOptionRow>

      <SettingsOptionRow data-testid="windows-embed-in-main-row">
        <div className="min-w-0">
          <label
            className="text-sm font-medium"
            htmlFor="windows-embed-in-main-switch"
          >
            Embed windows in the main Buzz session
          </label>
          <p
            className="text-sm font-normal text-muted-foreground/70"
            data-settings-subcopy
          >
            Open threads, playgrounds, and splits inside the main window
            instead of a separate OS window. Requires the Windows section.
          </p>
        </div>
        <Switch
          checked={embedEnabled}
          data-testid="windows-embed-in-main-toggle"
          disabled={!settings.showWindowsSection}
          id="windows-embed-in-main-switch"
          onCheckedChange={setEmbedInMain}
        />
      </SettingsOptionRow>
    </>
  );
}
