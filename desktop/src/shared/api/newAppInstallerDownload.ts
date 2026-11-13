import { Channel } from "@tauri-apps/api/core";

import { invokeTauri } from "@/shared/api/tauri";

/** Progress the backend sends while it downloads the new app's installer. */
export type InstallerDownloadEvent =
  | { kind: "started"; fileName: string }
  | { kind: "progress"; receivedBytes: number; totalBytes: number | null };

export type InstallerFailureReason =
  | "busy"
  | "rejected"
  | "network"
  | "redirect-blocked"
  | "too-large"
  | "storage"
  | "permission-denied"
  | "protection";

export type InstallerDownloadOutcome =
  | { status: "saved"; fileName: string; folder: string }
  | { status: "cancelled" }
  | { status: "failed"; reason: InstallerFailureReason };

/**
 * Downloads the new Buzz app's installer into the Downloads folder. The
 * backend validates `url` itself, names the file and picks the folder. It
 * resolves once the download is saved, cancelled or failed; only one runs at
 * a time.
 */
export function downloadNewAppInstaller(
  url: string,
  onEvent: (event: InstallerDownloadEvent) => void,
): Promise<InstallerDownloadOutcome> {
  return invokeTauri<InstallerDownloadOutcome>("download_new_app_installer", {
    url,
    onEvent: new Channel<InstallerDownloadEvent>(onEvent),
  });
}

/** Stops the running installer download, if any, and removes its partial file. */
export function cancelNewAppInstallerDownload(): Promise<void> {
  return invokeTauri<void>("cancel_new_app_installer_download");
}

/** Shows the last installer the app saved in the system file manager. */
export function revealNewAppInstaller(): Promise<void> {
  return invokeTauri<void>("reveal_new_app_installer");
}
