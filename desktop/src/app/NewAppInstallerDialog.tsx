import * as React from "react";

import {
  dialogDescription,
  dialogTitle,
  downloadPercent,
  downloadProgressText,
  type FailureAction,
  type FileManagerPlatform,
  failureActions,
  ignoreRepeatedEnter,
  type InstallerDialogState,
  liveAnnouncement,
  REVEAL_FAILED_MESSAGE,
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
  /** Hand the installer that failed to download to the browser. */
  onDownloadInBrowser: () => void;
  onOpenWebsite: () => void;
  onRetry: () => void;
  onReveal: () => void;
};

const FAILURE_BUTTONS: Record<
  FailureAction,
  { label: string; ariaLabel?: string; testId: string }
> = {
  retry: { label: "Try again", testId: "new-app-installer-retry" },
  browser: {
    label: "Download in browser",
    ariaLabel: "Download the installer in your browser",
    testId: "new-app-installer-browser",
  },
  website: {
    label: "Open buzz.xyz",
    ariaLabel: "Open buzz.xyz in your browser",
    testId: "new-app-installer-website",
  },
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
  onDownloadInBrowser,
  onOpenWebsite,
  onRetry,
  onReveal,
}: NewAppInstallerDialogProps) {
  const platform = fileManagerPlatform();
  // Keep the last open state on screen while the dialog animates out.
  const [shown, setShown] = React.useState(state);
  if (state.status !== "closed" && state !== shown) {
    setShown(state);
  }
  const isDownloading = state.status === "downloading";

  // The focused button goes away whenever the status changes: Cancel when
  // the download ends, "Try again" when it restarts. Move focus to the action
  // that replaces it instead of letting it fall to the dialog.
  const cancelRef = React.useRef<HTMLButtonElement>(null);
  const primaryActionRef = React.useRef<HTMLButtonElement>(null);
  const shownStatus = shown.status;
  const focusCurrentAction = React.useCallback(() => {
    const action =
      shownStatus === "downloading"
        ? cancelRef.current
        : primaryActionRef.current;
    action?.focus();
    return action !== null;
  }, [shownStatus]);
  React.useEffect(() => {
    focusCurrentAction();
  }, [focusCurrentAction]);

  const failure =
    shown.status === "failed" ? failureActions(shown.reason) : null;
  const failureHandlers: Record<FailureAction, () => void> = {
    retry: onRetry,
    browser: onDownloadInBrowser,
    website: onOpenWebsite,
  };
  const failureButton = (action: FailureAction, isPrimary: boolean) => (
    <Button
      aria-label={FAILURE_BUTTONS[action].ariaLabel}
      data-testid={FAILURE_BUTTONS[action].testId}
      key={action}
      onClick={failureHandlers[action]}
      ref={isPrimary ? primaryActionRef : undefined}
      type="button"
      variant={isPrimary ? "default" : "outline"}
    >
      {FAILURE_BUTTONS[action].label}
    </Button>
  );

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
        // Enter still held from the press that started the download repeats
        // onto the button focus has just moved to. It must not press it.
        onKeyDown={ignoreRepeatedEnter}
        onOpenAutoFocus={(event) => {
          if (focusCurrentAction()) {
            event.preventDefault();
          }
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
            {dialogDescription(shown, platform)}
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
              <p
                className="text-sm text-destructive"
                data-testid="new-app-installer-reveal-failed"
                role="alert"
              >
                {REVEAL_FAILED_MESSAGE}
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
          {liveAnnouncement(shown, platform)}
        </p>

        <DialogFooter>
          {shown.status === "downloading" ? (
            <Button
              data-testid="new-app-installer-cancel"
              disabled={shown.isCancelling}
              onClick={onCancel}
              ref={cancelRef}
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
                {revealLabel(platform)}
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
          {failure ? (
            <>
              {failure.secondary
                ? failureButton(failure.secondary, false)
                : null}
              {failureButton(failure.primary, true)}
            </>
          ) : null}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
