// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT, useUiLanguage } from "@/shared/i18n/uiTranslation";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useUpdaterContext } from "./hooks/UpdaterProvider";
import { Button } from "@/shared/ui/button";
import {
  SettingsOptionGroup,
  SettingsOptionRow,
} from "./ui/SettingsOptionGroup";
import { SettingsSectionHeader } from "./ui/SettingsSectionHeader";
export function UpdateChecker() {
  useUiLanguage();

  const { status, checkForUpdate, installAndRelaunch } = useUpdaterContext();

  return (
    <section className="min-w-0" data-testid="settings-updates">
      <SettingsSectionHeader
        title={uiT("Software Updates")}
        description={uiT(
          "Keep Buzz up to date with the latest features and fixes.",
        )}
      />

      <SettingsOptionGroup title={uiT("Update status")}>
        {status.state === "idle" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                {uiT("Check if a new version is available.")}
              </p>
            </div>
            <Button size="sm" onClick={checkForUpdate}>
              {uiT("Check for Updates")}
            </Button>
          </SettingsOptionRow>
        )}

        {status.state === "checking" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                {uiT("Checking for updates...")}
              </p>
            </div>
          </SettingsOptionRow>
        )}

        {status.state === "up-to-date" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                {uiT("You're on the latest version.")}
              </p>
            </div>
            <Button variant="outline" size="sm" onClick={checkForUpdate}>
              {uiT("Check Again")}
            </Button>
          </SettingsOptionRow>
        )}

        {status.state === "unavailable" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                {uiT(
                  "Automatic updates aren't available on this build. Download the latest release manually.",
                )}
              </p>
            </div>
            <Button variant="outline" size="sm" onClick={checkForUpdate}>
              {uiT("Check Again")}
            </Button>
          </SettingsOptionRow>
        )}

        {status.state === "manual-required" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p className="text-sm font-medium">
                {uiT("Update available — v{0}", { p0: status.version })}
              </p>
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                {uiT(
                  "In-app updates aren't supported on this Linux package. Download the new version from GitHub.",
                )}{" "}
                <span>
                  {uiT("Switch to the AppImage build for automatic updates.")}
                </span>
              </p>
            </div>
            <Button size="sm" onClick={() => void openUrl(status.releaseUrl)}>
              {uiT("Download Update")}
            </Button>
          </SettingsOptionRow>
        )}

        {status.state === "available" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                {uiT("Preparing update...")}
              </p>
            </div>
          </SettingsOptionRow>
        )}

        {status.state === "downloading" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                {uiT("Downloading update...")}
              </p>
            </div>
          </SettingsOptionRow>
        )}

        {status.state === "installing" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                {uiT("Installing update...")}
              </p>
            </div>
          </SettingsOptionRow>
        )}

        {status.state === "ready" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                {uiT("Update downloaded. Click to apply.")}
              </p>
            </div>
            <Button size="sm" onClick={installAndRelaunch}>
              {uiT("Update Now")}
            </Button>
          </SettingsOptionRow>
        )}

        {status.state === "error" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p className="text-sm font-normal text-destructive">
                {uiT("Update failed: {0}", { p0: status.message })}
              </p>
            </div>
            <Button variant="outline" size="sm" onClick={checkForUpdate}>
              {uiT("Retry")}
            </Button>
          </SettingsOptionRow>
        )}
      </SettingsOptionGroup>
    </section>
  );
}
