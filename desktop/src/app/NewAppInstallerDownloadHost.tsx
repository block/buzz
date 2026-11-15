import { openUrl } from "@tauri-apps/plugin-opener";

import { NewAppInstallerDialog } from "@/app/NewAppInstallerDialog";
import { BUZZ_APP_WEBSITE_URL } from "@/app/newAppInstaller";
import {
  newAppInstallerTriggerRef,
  useNewAppInstallerDownload,
} from "@/app/useNewAppInstallerDownload";

/**
 * Shows the in-app download of the new app's installer in its dialog.
 *
 * Mounted once for the main window, outside everything that unmounts with
 * the community, the identity or onboarding. The banner that starts a
 * download can go away while it runs: on sign-out, when a replaced identity
 * lands on onboarding, or when a community switch ends on the
 * connection-failed screen. The dialog and its Cancel button stay, so no
 * download ever runs without them.
 */
export function NewAppInstallerDownloadHost() {
  const { state, start, cancel, dismiss, reveal } =
    useNewAppInstallerDownload();

  return (
    <NewAppInstallerDialog
      onCancel={cancel}
      onDismiss={dismiss}
      onDownloadInBrowser={() => {
        // The URL the download was started with, which the resolver and
        // the backend both accepted. The browser downloads it its own way.
        if (state.status === "failed") {
          void openUrl(state.url);
        }
        dismiss();
      }}
      onOpenWebsite={() => {
        void openUrl(BUZZ_APP_WEBSITE_URL);
        dismiss();
      }}
      onRetry={() => {
        // From the state: the banner that resolved the URL may be gone.
        if (state.status === "failed") {
          start(state.url);
        }
      }}
      onReveal={reveal}
      returnFocusRef={newAppInstallerTriggerRef}
      state={state}
    />
  );
}
