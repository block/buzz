import { useUpdaterContext } from "./hooks/UpdaterProvider";
import { Button } from "@/shared/ui/button";
import {
  SettingsOptionGroup,
  SettingsOptionRow,
} from "./ui/SettingsOptionGroup";

export function UpdateChecker() {
  const { status, checkForUpdate, relaunch } = useUpdaterContext();

  return (
    <section className="min-w-0">
      <div className="mb-12 min-w-0">
        <h2 className="text-2xl font-semibold tracking-tight">
          Software Updates
        </h2>
        <p className="text-base font-normal text-muted-foreground">
          Keep Buzz up to date with the latest features and fixes.
        </p>
      </div>

      <SettingsOptionGroup>
        {status.state === "idle" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p className="text-sm font-medium">Update status</p>
              <p className="text-sm font-normal text-muted-foreground">
                Check if a new version is available.
              </p>
            </div>
            <Button size="sm" onClick={checkForUpdate}>
              Check for Updates
            </Button>
          </SettingsOptionRow>
        )}

        {status.state === "checking" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p className="text-sm font-medium">Update status</p>
              <p className="text-sm font-normal text-muted-foreground">
                Checking for updates...
              </p>
            </div>
          </SettingsOptionRow>
        )}

        {status.state === "up-to-date" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p className="text-sm font-medium">Update status</p>
              <p className="text-sm font-normal text-muted-foreground">
                You're on the latest version.
              </p>
            </div>
            <Button variant="outline" size="sm" onClick={checkForUpdate}>
              Check Again
            </Button>
          </SettingsOptionRow>
        )}

        {status.state === "unavailable" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p className="text-sm font-medium">Update status</p>
              <p className="text-sm font-normal text-muted-foreground">
                Automatic updates aren't available on this build. Download the
                latest release manually.
              </p>
            </div>
            <Button variant="outline" size="sm" onClick={checkForUpdate}>
              Check Again
            </Button>
          </SettingsOptionRow>
        )}

        {status.state === "available" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p className="text-sm font-medium">Update status</p>
              <p className="text-sm font-normal text-muted-foreground">
                Preparing update...
              </p>
            </div>
          </SettingsOptionRow>
        )}

        {status.state === "downloading" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p className="text-sm font-medium">Update status</p>
              <p className="text-sm font-normal text-muted-foreground">
                Downloading update...
              </p>
            </div>
          </SettingsOptionRow>
        )}

        {status.state === "installing" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p className="text-sm font-medium">Update status</p>
              <p className="text-sm font-normal text-muted-foreground">
                Installing update...
              </p>
            </div>
          </SettingsOptionRow>
        )}

        {status.state === "ready" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p className="text-sm font-medium">Update status</p>
              <p className="text-sm font-normal text-muted-foreground">
                Update installed. Restart to apply.
              </p>
            </div>
            <Button size="sm" onClick={relaunch}>
              Restart Now
            </Button>
          </SettingsOptionRow>
        )}

        {status.state === "error" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p className="text-sm font-medium">Update status</p>
              <p className="text-sm font-normal text-destructive">
                Update failed: {status.message}
              </p>
            </div>
            <Button variant="outline" size="sm" onClick={checkForUpdate}>
              Retry
            </Button>
          </SettingsOptionRow>
        )}
      </SettingsOptionGroup>
    </section>
  );
}
