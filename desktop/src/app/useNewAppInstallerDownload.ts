import * as React from "react";

import {
  INSTALLER_DIALOG_CLOSED,
  type InstallerDialogAction,
  type InstallerDialogState,
  reduceInstallerDialog,
} from "@/app/newAppInstallerDownload";
import {
  cancelNewAppInstallerDownload,
  downloadNewAppInstaller,
  revealNewAppInstaller,
} from "@/shared/api/newAppInstallerDownload";

// The download belongs to the app, not to a community, so its state lives
// outside React and outside `resetCommunityState()`. The banner that starts
// it comes and goes with the community, the identity and onboarding; the
// dialog that shows it is mounted once, above all of that.
let state: InstallerDialogState = INSTALLER_DIALOG_CLOSED;
// Set synchronously, so two clicks in one tick cannot both start a download.
let isRunning = false;
// Fences the progress events of an earlier attempt, which can arrive after
// that attempt has settled and the next one has started.
let attempt = 0;
const listeners = new Set<() => void>();

function dispatch(action: InstallerDialogAction) {
  const next = reduceInstallerDialog(state, action);
  if (next === state) {
    return;
  }
  state = next;
  for (const listener of listeners) {
    listener();
  }
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getState() {
  return state;
}

function reveal() {
  revealNewAppInstaller().then(
    () => dispatch({ type: "revealSettled", ok: true }),
    (error: unknown) => {
      console.warn("Could not show the new app installer:", error);
      dispatch({ type: "revealSettled", ok: false });
    },
  );
}

/**
 * The control that offers the download: the banner's button while a banner is
 * mounted, null otherwise. The dialog returns focus to it when it closes.
 */
export const newAppInstallerTriggerRef: React.RefObject<HTMLButtonElement | null> =
  { current: null };

/**
 * Starts the in-app download of the installer at `url`, which
 * `NewAppInstallerDownloadHost` then shows. Does nothing while one runs.
 */
export function startNewAppInstallerDownload(url: string) {
  if (isRunning) {
    return;
  }
  isRunning = true;
  attempt += 1;
  const thisAttempt = attempt;
  dispatch({ type: "started", url });
  downloadNewAppInstaller(url, (event) => {
    if (attempt === thisAttempt) {
      dispatch({ type: "event", event });
    }
  })
    .catch((error: unknown) => {
      console.warn("The new app installer download failed:", error);
      return { status: "failed", reason: "unknown" } as const;
    })
    .then((outcome) => {
      isRunning = false;
      dispatch({ type: "settled", outcome });
      if (outcome.status === "saved") {
        reveal();
      }
    });
}

function cancel() {
  dispatch({ type: "cancelRequested" });
  cancelNewAppInstallerDownload().catch((error: unknown) => {
    console.warn("Could not cancel the new app installer download:", error);
    dispatch({ type: "cancelFailed" });
  });
}

function dismiss() {
  dispatch({ type: "dismissed" });
}

/**
 * Holds the state the installer download's dialog shows, and the actions the
 * dialog offers. One download at a time, for the whole app.
 *
 * `NewAppInstallerDownloadHost` is the one place that mounts this, once per
 * main window and above everything that remounts, so a running download
 * always has a dialog with a Cancel button. The download does not depend on
 * the component that started it, and the backend stops it when the main
 * window is destroyed or the app exits.
 *
 * Mounting stops a download this page knows nothing about, which is one left
 * running by a page that was reloaded. That assumes a single window mounts
 * this hook: a second window would cancel the first window's download on
 * mount, so the huddle window must not mount it.
 */
export function useNewAppInstallerDownload() {
  const current = React.useSyncExternalStore(subscribe, getState);

  React.useEffect(() => {
    if (!isRunning) {
      cancelNewAppInstallerDownload().catch(() => {});
    }
  }, []);

  return {
    state: current,
    start: startNewAppInstallerDownload,
    cancel,
    dismiss,
    reveal,
  };
}
