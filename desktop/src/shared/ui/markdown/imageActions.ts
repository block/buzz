// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
import { toast } from "sonner";

import { invokeTauri } from "@/shared/api/tauri";

export function copyImageToClipboard(src: string | undefined) {
  if (!src) return;
  invokeTauri("copy_image_to_clipboard", { url: src })
    .then(() => toast.success(uiT("Copied to clipboard")))
    .catch((error: unknown) => {
      toast.error(error instanceof Error ? error.message : uiT("Copy failed"));
    });
}

export function downloadImage(src: string | undefined) {
  if (!src) return;
  invokeTauri("download_image", { url: src }).catch((error: unknown) => {
    toast.error(
      error instanceof Error ? error.message : uiT("Download failed"),
    );
  });
}

/** Built-in fallback for attachments whose author supplied no caption. */
export function mediaFallbackLabel(isVideo: boolean): string {
  return isVideo ? uiT("Video attachment") : uiT("Image attachment");
}

/** Accessible zoom action; author-provided alternative text remains literal. */
export function imageZoomLabel(alt: string | undefined): string {
  return alt?.trim() ? uiT("Zoom image: {0}", { p0: alt }) : uiT("Zoom image");
}
