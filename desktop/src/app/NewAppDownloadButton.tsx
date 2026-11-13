import { openUrl } from "@tauri-apps/plugin-opener";

import { BUZZ_APP_WEBSITE_URL } from "@/app/newAppInstaller";
import { ignoreRepeatedEnter } from "@/app/newAppInstallerDownload";
import {
  newAppInstallerTriggerRef,
  startNewAppInstallerDownload,
} from "@/app/useNewAppInstallerDownload";
import { Button } from "@/shared/ui/button";

type NewAppDownloadButtonProps = {
  className?: string;
  /** This machine's installer, or null while it is unknown or unavailable. */
  installerUrl: string | null;
};

/**
 * The banner's "Upgrade" button. With an installer URL it starts the download
 * inside the app, which `NewAppInstallerDownloadHost` shows in a dialog;
 * without one it opens the website in the browser. The click never waits on
 * anything.
 */
export function NewAppDownloadButton({
  className,
  installerUrl,
}: NewAppDownloadButtonProps) {
  const downloadsInApp = installerUrl !== null;

  return (
    <Button
      aria-haspopup={downloadsInApp ? "dialog" : undefined}
      aria-label={
        downloadsInApp
          ? "Upgrade, downloads the installer"
          : "Upgrade, opens in your browser"
      }
      className={className}
      data-download-source={downloadsInApp ? "installer" : "fallback"}
      data-testid="deprecation-banner-download"
      onClick={() => {
        if (installerUrl) {
          startNewAppInstallerDownload(installerUrl);
        } else {
          void openUrl(BUZZ_APP_WEBSITE_URL);
        }
      }}
      // Closing the dialog with Enter returns focus here with the key still
      // down. Its repeat must not start the download again.
      onKeyDown={ignoreRepeatedEnter}
      ref={newAppInstallerTriggerRef}
      size="sm"
      type="button"
      variant="outline"
    >
      Upgrade
    </Button>
  );
}
