import { toast } from "sonner";

import { copyTextToSystemClipboard } from "@/shared/api/tauriMedia";

/**
 * Write text through the native clipboard integration.
 *
 * `html` is an optional richer flavor written in the same clipboard
 * transaction. External apps read `text`; Buzz reads `html` on paste to
 * recover metadata the plain flavor deliberately omits (mention pubkeys).
 */
export async function writeTextToClipboard(
  text: string,
  html?: string,
): Promise<void> {
  await copyTextToSystemClipboard(text, html);
}

/** Copy text and show standard success/error feedback. */
export function copyTextToClipboard(
  text: string,
  successMessage = "Copied to clipboard",
  html?: string,
) {
  void writeTextToClipboard(text, html)
    .then(() => {
      toast.success(successMessage);
    })
    .catch(() => {
      toast.error("Failed to copy to clipboard");
    });
}
