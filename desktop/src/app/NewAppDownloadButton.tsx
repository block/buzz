import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { NewAppInstallerDialog } from "@/app/NewAppInstallerDialog";
import { BUZZ_APP_WEBSITE_URL } from "@/app/newAppInstaller";
import { useNewAppInstallerDownload } from "@/app/useNewAppInstallerDownload";
import { Button } from "@/shared/ui/button";

type NewAppDownloadButtonProps = {
  className?: string;
  /** This machine's installer, or null while it is unknown or unavailable. */
  installerUrl: string | null;
};

/**
 * The banner's "Upgrade" button. With an installer URL it downloads
 * the installer inside the app and shows the progress in a dialog; without
 * one it opens the website in the browser. The click never waits on anything.
 */
export function NewAppDownloadButton({
  className,
  installerUrl,
}: NewAppDownloadButtonProps) {
  const { state, start, cancel, dismiss, reveal } =
    useNewAppInstallerDownload();
  const buttonRef = React.useRef<HTMLButtonElement>(null);
  const downloadsInApp = installerUrl !== null;

  return (
    <>
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
            start(installerUrl);
          } else {
            void openUrl(BUZZ_APP_WEBSITE_URL);
          }
        }}
        ref={buttonRef}
        size="sm"
        type="button"
        variant="outline"
      >
        Upgrade
      </Button>
      <NewAppInstallerDialog
        onCancel={cancel}
        onDismiss={dismiss}
        onOpenWebsite={() => {
          void openUrl(BUZZ_APP_WEBSITE_URL);
          dismiss();
        }}
        onRetry={() => {
          if (installerUrl) {
            start(installerUrl);
          }
        }}
        onReveal={reveal}
        returnFocusRef={buttonRef}
        state={state}
      />
    </>
  );
}
