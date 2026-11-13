import * as React from "react";

import {
  INSTALLER_DIALOG_CLOSED,
  reduceInstallerDialog,
} from "@/app/newAppInstallerDownload";
import {
  cancelNewAppInstallerDownload,
  downloadNewAppInstaller,
  revealNewAppInstaller,
} from "@/shared/api/newAppInstallerDownload";

/**
 * Runs the in-app download of the new app's installer and holds the state its
 * dialog shows. One download at a time; unmounting cancels a running one.
 */
export function useNewAppInstallerDownload() {
  const [state, dispatch] = React.useReducer(
    reduceInstallerDialog,
    INSTALLER_DIALOG_CLOSED,
  );
  // Set synchronously, so two clicks in one tick cannot both start a download.
  const isRunningRef = React.useRef(false);
  // Fences events and results of an earlier attempt, or of an unmounted hook.
  const attemptRef = React.useRef(0);

  const reveal = React.useCallback(() => {
    revealNewAppInstaller().then(
      () => dispatch({ type: "revealSettled", ok: true }),
      (error: unknown) => {
        console.warn("Could not show the new app installer:", error);
        dispatch({ type: "revealSettled", ok: false });
      },
    );
  }, []);

  const start = React.useCallback(
    (url: string) => {
      if (isRunningRef.current) {
        return;
      }
      isRunningRef.current = true;
      attemptRef.current += 1;
      const attempt = attemptRef.current;
      dispatch({ type: "started" });
      downloadNewAppInstaller(url, (event) => {
        if (attemptRef.current === attempt) {
          dispatch({ type: "event", event });
        }
      })
        .catch((error: unknown) => {
          console.warn("The new app installer download failed:", error);
          return { status: "failed", reason: "unknown" } as const;
        })
        .then((outcome) => {
          if (attemptRef.current !== attempt) {
            return;
          }
          isRunningRef.current = false;
          dispatch({ type: "settled", outcome });
          if (outcome.status === "saved") {
            reveal();
          }
        });
    },
    [reveal],
  );

  const cancel = React.useCallback(() => {
    dispatch({ type: "cancelRequested" });
    cancelNewAppInstallerDownload().catch((error: unknown) => {
      console.warn("Could not cancel the new app installer download:", error);
      dispatch({ type: "cancelFailed" });
    });
  }, []);

  const dismiss = React.useCallback(() => dispatch({ type: "dismissed" }), []);

  React.useEffect(() => {
    // A download left over from an earlier page lifetime has no dialog to
    // show it or cancel it, so it is stopped here.
    cancelNewAppInstallerDownload().catch(() => {});
    return () => {
      attemptRef.current += 1;
      if (isRunningRef.current) {
        isRunningRef.current = false;
        cancelNewAppInstallerDownload().catch(() => {});
      }
    };
  }, []);

  return { state, start, cancel, dismiss, reveal };
}
