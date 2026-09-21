import { toast } from "sonner";

import { i18n } from "@/i18n";
import { invokeTauri } from "@/shared/api/tauri";

export function copyImageToClipboard(src: string | undefined) {
  if (!src) return;
  invokeTauri("copy_image_to_clipboard", { url: src })
    .then(() => toast.success(i18n.t("shared.markdown.image.copied")))
    .catch((error: unknown) => {
      toast.error(
        error instanceof Error
          ? error.message
          : i18n.t("shared.markdown.image.copy-failed"),
      );
    });
}

export function downloadImage(src: string | undefined) {
  if (!src) return;
  invokeTauri("download_image", { url: src }).catch((error: unknown) => {
    toast.error(
      error instanceof Error
        ? error.message
        : i18n.t("shared.markdown.image.download-failed"),
    );
  });
}
