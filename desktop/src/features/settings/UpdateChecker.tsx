import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "@/i18n";
import { useUpdaterContext } from "./hooks/UpdaterProvider";
import { Button } from "@/shared/ui/button";
import {
  SettingsOptionGroup,
  SettingsOptionRow,
} from "./ui/SettingsOptionGroup";
import { SettingsSectionHeader } from "./ui/SettingsSectionHeader";
export function UpdateChecker() {
  const { t } = useTranslation();
  const { status, checkForUpdate, installAndRelaunch } = useUpdaterContext();

  return (
    <section className="min-w-0" data-testid="settings-updates">
      <SettingsSectionHeader
        title={t("settings.updates.software-updates")}
        description={t("settings.updates.section-description")}
      />

      <SettingsOptionGroup title={t("settings.updates.status-group")}>
        {status.state === "idle" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                {t("settings.updates.idle-hint")}
              </p>
            </div>
            <Button size="sm" onClick={checkForUpdate}>
              {t("settings.updates.check")}
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
                {t("settings.updates.checking")}
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
                {t("settings.updates.up-to-date")}
              </p>
            </div>
            <Button variant="outline" size="sm" onClick={checkForUpdate}>
              {t("settings.updates.check-again")}
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
                {t("settings.updates.unavailable-hint")}
              </p>
            </div>
            <Button variant="outline" size="sm" onClick={checkForUpdate}>
              {t("settings.updates.check-again")}
            </Button>
          </SettingsOptionRow>
        )}

        {status.state === "manual-required" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p className="text-sm font-medium">
                {t("settings.updates.available-version", {
                  version: status.version,
                })}
              </p>
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                {t("settings.updates.manual-required-hint")}{" "}
                <span>{t("settings.updates.manual-required-appimage")}</span>
              </p>
            </div>
            <Button size="sm" onClick={() => void openUrl(status.releaseUrl)}>
              {t("settings.updates.download-update")}
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
                {t("settings.updates.preparing")}
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
                {t("settings.updates.downloading-dots")}
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
                {t("settings.updates.installing-dots")}
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
                {t("settings.updates.ready-hint")}
              </p>
            </div>
            <Button size="sm" onClick={installAndRelaunch}>
              {t("settings.updates.update-now")}
            </Button>
          </SettingsOptionRow>
        )}

        {status.state === "error" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p className="text-sm font-normal text-destructive">
                {t("settings.updates.failed", { message: status.message })}
              </p>
            </div>
            <Button variant="outline" size="sm" onClick={checkForUpdate}>
              {t("settings.updates.retry")}
            </Button>
          </SettingsOptionRow>
        )}
      </SettingsOptionGroup>
    </section>
  );
}
