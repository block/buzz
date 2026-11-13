import type {
  InstallerDownloadEvent,
  InstallerDownloadOutcome,
  InstallerFailureReason,
} from "@/shared/api/newAppInstallerDownload";

/** `unknown` covers a download command that failed without an outcome. */
export type InstallerDialogFailure = InstallerFailureReason | "unknown";

export type InstallerDialogState =
  | { status: "closed" }
  | {
      status: "downloading";
      /** The installer being downloaded. */
      url: string;
      /** Null until the backend has accepted the URL and named the file. */
      fileName: string | null;
      receivedBytes: number;
      /** Null when the server did not say how big the file is. */
      totalBytes: number | null;
      isCancelling: boolean;
    }
  | {
      status: "finished";
      fileName: string;
      folder: string;
      revealFailed: boolean;
    }
  | {
      status: "failed";
      reason: InstallerDialogFailure;
      /** The installer that failed to download, for a retry or the browser. */
      url: string;
    };

export type InstallerDialogAction =
  | { type: "started"; url: string }
  | { type: "event"; event: InstallerDownloadEvent }
  | {
      type: "settled";
      outcome:
        | InstallerDownloadOutcome
        | { status: "failed"; reason: "unknown" };
    }
  | { type: "cancelRequested" }
  | { type: "cancelFailed" }
  | { type: "revealSettled"; ok: boolean }
  | { type: "dismissed" };

export const INSTALLER_DIALOG_CLOSED: InstallerDialogState = {
  status: "closed",
};

export function reduceInstallerDialog(
  state: InstallerDialogState,
  action: InstallerDialogAction,
): InstallerDialogState {
  switch (action.type) {
    case "started":
      // A download that is already running is never restarted.
      if (state.status === "downloading") {
        return state;
      }
      return {
        status: "downloading",
        url: action.url,
        fileName: null,
        receivedBytes: 0,
        totalBytes: null,
        isCancelling: false,
      };
    case "event":
      if (state.status !== "downloading") {
        return state;
      }
      if (action.event.kind === "started") {
        return { ...state, fileName: action.event.fileName };
      }
      return {
        ...state,
        // Channel messages can arrive out of order; progress never goes back.
        receivedBytes: Math.max(
          state.receivedBytes,
          action.event.receivedBytes,
        ),
        totalBytes: action.event.totalBytes,
      };
    case "settled":
      if (state.status !== "downloading") {
        return state;
      }
      if (action.outcome.status === "saved") {
        // Also when the user had asked to cancel: the file exists, so say so.
        return {
          status: "finished",
          fileName: action.outcome.fileName,
          folder: action.outcome.folder,
          revealFailed: false,
        };
      }
      if (action.outcome.status === "cancelled" || state.isCancelling) {
        return INSTALLER_DIALOG_CLOSED;
      }
      return {
        status: "failed",
        reason: action.outcome.reason,
        url: state.url,
      };
    case "cancelRequested":
      return state.status === "downloading"
        ? { ...state, isCancelling: true }
        : state;
    case "cancelFailed":
      // Give the Cancel button back so the user can ask again.
      return state.status === "downloading"
        ? { ...state, isCancelling: false }
        : state;
    case "revealSettled":
      return state.status === "finished"
        ? { ...state, revealFailed: !action.ok }
        : state;
    case "dismissed":
      // A running download is cancelled, never just hidden.
      return state.status === "downloading" ? state : INSTALLER_DIALOG_CLOSED;
  }
}

/** Whole percent done, or null when the total is unknown. */
export function downloadPercent(
  receivedBytes: number,
  totalBytes: number | null,
): number | null {
  if (totalBytes === null || totalBytes <= 0) {
    return null;
  }
  return Math.min(
    100,
    Math.max(0, Math.floor((receivedBytes / totalBytes) * 100)),
  );
}

const KIB = 1024;
const MIB = KIB * 1024;
const GIB = MIB * 1024;

export function formatBytes(bytes: number): string {
  if (bytes < KIB) {
    return `${bytes} B`;
  }
  if (bytes < MIB) {
    return `${Math.round(bytes / KIB)} KB`;
  }
  if (bytes < GIB) {
    return `${(bytes / MIB).toFixed(1)} MB`;
  }
  return `${(bytes / GIB).toFixed(2)} GB`;
}

/** "39.5 MB of 79.0 MB (50%)", or "39.5 MB downloaded" without a total. */
export function downloadProgressText(
  receivedBytes: number,
  totalBytes: number | null,
): string {
  const percent = downloadPercent(receivedBytes, totalBytes);
  if (totalBytes === null || percent === null) {
    return `${formatBytes(receivedBytes)} downloaded`;
  }
  return `${formatBytes(receivedBytes)} of ${formatBytes(totalBytes)} (${percent}%)`;
}

/** The last segment of a folder path, whichever separator the platform uses. */
export function folderName(folder: string): string {
  const segments = folder.split(/[\\/]/).filter((segment) => segment !== "");
  return segments.at(-1) ?? folder;
}

export type FileManagerPlatform = "macos" | "windows" | "linux";

export function revealLabel(platform: FileManagerPlatform): string {
  switch (platform) {
    case "macos":
      return "Show in Finder";
    case "windows":
      return "Show in File Explorer";
    case "linux":
      return "Show in folder";
  }
}

const FAILURE_MESSAGES: Record<
  Exclude<InstallerDialogFailure, "permission-denied">,
  string
> = {
  busy: "Another download of the new app is still running. Give it a moment, then try again.",
  rejected:
    "Buzz couldn't use the download link for this computer, so nothing was downloaded.",
  network:
    "The download stopped before it was complete. Check your internet connection and try again.",
  "redirect-blocked":
    "The download was sent to a site Buzz doesn't recognise, so Buzz stopped it.",
  "too-large":
    "The file was much bigger than the installer should be, so the download was stopped.",
  storage:
    "The installer couldn't be saved to your Downloads folder. Check that there is free space and try again.",
  protection:
    "Buzz couldn't mark the installer as downloaded from the internet, so it didn't keep the file.",
  unknown: "Something went wrong while downloading the installer.",
};

export function failureMessage(
  reason: InstallerDialogFailure,
  platform: FileManagerPlatform,
): string {
  if (reason !== "permission-denied") {
    return FAILURE_MESSAGES[reason];
  }
  // On macOS this is what declining the system's "access files in your
  // Downloads folder" prompt leads to, and the prompt is not shown again.
  return platform === "macos"
    ? "Buzz isn't allowed to save to your Downloads folder. To allow it, open System Settings, choose Privacy & Security, then Files and Folders, and turn on Downloads Folder for Buzz."
    : "Buzz isn't allowed to save to your Downloads folder. Check that your account can add files to that folder.";
}

/** What a failed download offers, and which of the two takes focus. */
export type FailureAction = "retry" | "browser" | "website";

/**
 * "Try again" leads only where trying again can work. A failure that would
 * repeat sends the same installer to the browser, which downloads it its own
 * way; a link Buzz refused is never opened, so that one leads to the website.
 */
export function failureActions(reason: InstallerDialogFailure): {
  primary: FailureAction;
  secondary: FailureAction | null;
} {
  switch (reason) {
    case "busy":
    case "network":
    case "storage":
    case "unknown":
      return { primary: "retry", secondary: "website" };
    case "permission-denied":
      // Once access is granted, the same download works.
      return { primary: "browser", secondary: "retry" };
    case "redirect-blocked":
    case "too-large":
    case "protection":
      return { primary: "browser", secondary: "website" };
    case "rejected":
      return { primary: "website", secondary: null };
  }
}

const FAILURE_NEXT_STEP: Record<FailureAction, string> = {
  retry: "You can also get the new app from buzz.xyz.",
  browser: "You can download the installer in your browser instead.",
  website: "You can get the new app from buzz.xyz.",
};

export function dialogTitle(state: InstallerDialogState): string {
  switch (state.status) {
    case "finished":
      return "The new Buzz app is downloaded";
    case "failed":
      return "The download didn't finish";
    default:
      return "Downloading the new Buzz app";
  }
}

export function dialogDescription(
  state: InstallerDialogState,
  platform: FileManagerPlatform,
): string {
  switch (state.status) {
    case "closed":
      return "";
    case "downloading":
      if (state.isCancelling) {
        return "Cancelling the download…";
      }
      return state.fileName
        ? `Saving ${state.fileName} to your Downloads folder.`
        : "Starting the download…";
    case "finished": {
      const where = `${state.fileName} is in your ${folderName(state.folder)} folder.`;
      // An AppImage is saved like any download, so it cannot run yet.
      return state.fileName.endsWith(".AppImage")
        ? `${where} Before you open it, make it executable: in your file manager, open its Properties and allow it to run as a program, or run chmod +x on it in a terminal.`
        : `${where} Open it when you're ready to install.`;
    }
    case "failed":
      return `${failureMessage(state.reason, platform)} ${FAILURE_NEXT_STEP[failureActions(state.reason).primary]}`;
  }
}

/** Shown when the file manager did not open; the file itself is fine. */
export const REVEAL_FAILED_MESSAGE =
  "Buzz couldn't open the folder. The installer is in the folder shown above.";

/**
 * What a screen reader is told. It changes when the download starts, at each
 * quarter of the way, and when it finishes, fails or is being cancelled, so a
 * live region holding it speaks a handful of times and not on every tick.
 */
export function liveAnnouncement(
  state: InstallerDialogState,
  platform: FileManagerPlatform,
): string {
  switch (state.status) {
    case "closed":
      return "";
    case "downloading": {
      if (state.isCancelling) {
        return "Cancelling the download.";
      }
      if (!state.fileName) {
        return "Starting the download.";
      }
      const percent = downloadPercent(state.receivedBytes, state.totalBytes);
      const quarter = percent === null ? 0 : Math.floor(percent / 25) * 25;
      return quarter === 0
        ? `Downloading ${state.fileName}.`
        : `Downloading ${state.fileName}, ${quarter}% done.`;
    }
    case "finished":
      return `Download finished. ${state.fileName} is in your ${folderName(state.folder)} folder.`;
    case "failed":
      return `The download didn't finish. ${failureMessage(state.reason, platform)}`;
  }
}

/**
 * Keeps a held Enter from pressing a button again. The press that closes the
 * dialog, or that starts a download, moves focus to another button while the
 * key is still down, and the key's repeat must not press that one too.
 */
export function ignoreRepeatedEnter(event: {
  key: string;
  repeat: boolean;
  preventDefault: () => void;
}) {
  if (event.repeat && event.key === "Enter") {
    event.preventDefault();
  }
}
