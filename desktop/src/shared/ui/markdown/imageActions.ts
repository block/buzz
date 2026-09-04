import { toast } from "sonner";

import { invokeTauri } from "@/shared/api/tauri";

export function copyImageToClipboard(src: string | undefined) {
  if (!src) return;
  invokeTauri("copy_image_to_clipboard", { url: src })
    .then(() => toast.success("Copied to clipboard"))
    .catch((error: unknown) => {
      toast.error(error instanceof Error ? error.message : "Copy failed");
    });
}

export function downloadImage(src: string | undefined) {
  if (!src) return;
  invokeTauri("download_image", { url: src }).catch((error: unknown) => {
    toast.error(error instanceof Error ? error.message : "Download failed");
  });
}
