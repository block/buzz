import { toast } from "sonner";

import { invokeTauri } from "@/shared/api/tauri";

/**
 * Save an attachment through the native download command.
 *
 * Not an `<a download>` link: that navigates the webview to the blob URL,
 * which escapes to the OS browser and lands on a corporate CDN interstitial.
 * The Rust command fetches inside the app's tunnel and opens a save dialog.
 */
export function downloadAttachment(url: string, filename: string): void {
  invokeTauri("download_file", { filename, url }).catch((error: unknown) => {
    toast.error(error instanceof Error ? error.message : "Download failed");
  });
}
