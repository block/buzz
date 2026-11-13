import * as React from "react";

import {
  dialogDescription,
  dialogTitle,
  downloadPercent,
  downloadProgressText,
  type FileManagerPlatform,
  type InstallerDialogState,
  liveAnnouncement,
  revealLabel,
} from "@/app/newAppInstallerDownload";
import { isMacPlatform, isWindowsPlatform } from "@/shared/lib/platform";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Progress } from "@/shared/ui/progress";

function fileManagerPlatform(): FileManagerPlatform {
  if (isMacPlatform()) {
    return "macos";
  }
  return isWindowsPlatform() ? "windows" : "linux";
}

type NewAppInstallerDialogProps = {
  state: InstallerDialogState;
  /** The control that opened the dialog; focus returns to it on close. */
  returnFocusRef: React.RefObject<HTMLElement | null>;
  /** Stop the running download. The dialog closes once it has stopped. */
  onCancel: () => void;
  onDismiss: () => void;
  onOpenWebsite: () => void;
  onRetry: () => void;
  onReveal: () => void;
};

/**
 * Shows the in-app download of the new app's installer: progress while it
 * runs, where the file is once it is saved, and what to do when it fails.
 */
export function NewAppInstallerDialog({
  state,
  returnFocusRef,
  onCancel,
  onDismiss,
  onOpenWebsite,
  onRetry,
  onReveal,
}: NewAppInstallerDialogProps) {
  // Keep the last open state on screen while the dialog animates out.
  const [shown, setShown] = React.useState(state);
  if (state.status !== "closed" && state !== shown) {
    setShown(state);
  }
  const isDownloading = state.status === "downloading";

  // The focused Cancel button goes away when the download ends; move focus to
  // the action that replaces it instead of letting it fall to the dialog.
  const primaryActionRef = React.useRef<HTMLButtonElement>(null);
  const shownStatus = shown.status;
  React.useEffect(() => {
    if (shownStatus === "finished" || shownStatus === "failed") {
      primaryActionRef.current?.focus();
    }
  }, [shownStatus]);

  return (
    <Dialog
      onOpenChange={(open) => {
        if (open) {
          return;
        }
        // Escape and the close button cancel a running download.
        if (isDownloading) {
          onCancel();
        } else {
          onDismiss();
        }
      }}
      open={state.status !== "closed"}
    >
      <DialogContent
        className="max-w-md"
        data-status={shown.status}
        data-testid="new-app-installer-dialog"
        onCloseAutoFocus={(event) => {
          // Without a DialogTrigger the dialog would drop focus on the page.
          event.preventDefault();
          returnFocusRef.current?.focus();
        }}
        onInteractOutside={(event) => {
          // A stray click beside the dialog must not cancel the download.
          if (isDownloading) {
            event.preventDefault();
          }
        }}
      >
        <DialogHeader className="pr-8">
          <DialogTitle>{dialogTitle(shown)}</DialogTitle>
          <DialogDescription data-testid="new-app-installer-description">
            {dialogDescription(shown)}
          </DialogDescription>
        </DialogHeader>

        {shown.status === "downloading" ? (
          <div className="grid gap-2">
            <Progress
              aria-label="Download progress"
              aria-valuetext={downloadProgressText(
                shown.receivedBytes,
                shown.totalBytes,
              )}
              data-testid="new-app-installer-progress"
              value={downloadPercent(shown.receivedBytes, shown.totalBytes)}
            />
            <p
              className="text-xs text-muted-foreground tabular-nums"
              data-testid="new-app-installer-progress-text"
            >
              {downloadProgressText(shown.receivedBytes, shown.totalBytes)}
            </p>
          </div>
        ) : null}

        {shown.status === "finished" ? (
          <div className="grid gap-2">
            <p
              className="break-all font-mono text-xs text-muted-foreground"
              data-testid="new-app-installer-path"
            >
              {shown.folder}
            </p>
            {shown.revealFailed ? (
              <p className="text-sm text-destructive" role="alert">
                Buzz couldn't show the file. It may have been moved or deleted.
              </p>
            ) : null}
          </div>
        ) : null}

        {/* The only live region: it changes at milestones, not on every tick. */}
        <p
          aria-live="polite"
          className="sr-only"
          data-testid="new-app-installer-announcement"
          role="status"
        >
          {liveAnnouncement(shown)}
        </p>

        <DialogFooter>
          {shown.status === "downloading" ? (
            <Button
              data-testid="new-app-installer-cancel"
              disabled={shown.isCancelling}
              onClick={onCancel}
              type="button"
              variant="outline"
            >
              Cancel
            </Button>
          ) : null}
          {shown.status === "finished" ? (
            <>
              <Button
                data-testid="new-app-installer-reveal"
                onClick={onReveal}
                type="button"
                variant="outline"
              >
                {revealLabel(fileManagerPlatform())}
              </Button>
              <Button
                data-testid="new-app-installer-done"
                onClick={onDismiss}
                ref={primaryActionRef}
                type="button"
              >
                Done
              </Button>
            </>
          ) : null}
          {shown.status === "failed" ? (
            <>
              <Button
                aria-label="Open buzz.xyz in your browser"
                data-testid="new-app-installer-website"
                onClick={onOpenWebsite}
                type="button"
                variant="outline"
              >
                Open buzz.xyz
              </Button>
              <Button
                data-testid="new-app-installer-retry"
                onClick={onRetry}
                ref={primaryActionRef}
                type="button"
              >
                Try again
              </Button>
            </>
          ) : null}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
