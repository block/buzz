import { toast } from "sonner";

import { copyTextToSystemClipboard } from "@/shared/api/tauriMedia";

/** Write plain text through the native clipboard integration. */
export async function writeTextToClipboard(text: string): Promise<void> {
  await copyTextToSystemClipboard(text);
}

/** Copy plain text and show standard success/error feedback. */
export function copyTextToClipboard(
  text: string,
  successMessage = "Copied to clipboard",
) {
  void writeTextToClipboard(text)
    .then(() => {
      toast.success(successMessage);
    })
    .catch(() => {
      toast.error("Failed to copy to clipboard");
    });
}
