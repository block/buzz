import { toast } from "sonner";

/**
 * Copy plain text to the clipboard with a success toast, surfacing
 * `writeText` rejections (permissions, unfocused document) as an error
 * toast instead of an unhandled rejection.
 */
export function copyTextToClipboard(
  text: string,
  successMessage = "Copied to clipboard",
) {
  void navigator.clipboard
    .writeText(text)
    .then(() => {
      toast.success(successMessage);
    })
    .catch(() => {
      toast.error("Failed to copy to clipboard");
    });
}
