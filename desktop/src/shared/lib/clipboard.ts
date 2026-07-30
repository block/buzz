import { toast } from "sonner";

import { copyTextToSystemClipboard } from "@/shared/api/tauriMedia";

/** Write plain text through the native clipboard integration. */
export async function writeTextToClipboard(text: string): Promise<void> {
  await copyTextToSystemClipboard(text);
}

/**
 * Copy text trying navigator.clipboard.writeText first (works in
 * WKWebView/WebView2 from a user gesture and avoids the arboard path
 * that silently no-ops on Wayland), falling back to the native Tauri
 * clipboard.
 */
export async function copyTextWithFallback(text: string): Promise<void> {
  try {
    if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return;
    }
  } catch (error) {
    console.warn("navigator.clipboard.writeText failed", error);
  }
  await writeTextToClipboard(text);
}

/** Copy plain text and show standard success/error feedback. */
export function copyTextToClipboard(
  text: string,
  successMessage = "Copied to clipboard",
) {
  void copyTextWithFallback(text)
    .then(() => {
      toast.success(successMessage);
    })
    .catch(() => {
      toast.error("Failed to copy to clipboard");
    });
}
