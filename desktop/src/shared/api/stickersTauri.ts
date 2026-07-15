import { invokeTauri, type BlobDescriptor } from "@/shared/api/tauri";
import type { ImportedStickerDraft } from "@/shared/api/stickers";

export async function pickAndUploadStickerImage(
  coverOnly = false,
): Promise<BlobDescriptor | null> {
  return invokeTauri<BlobDescriptor | null>("pick_and_upload_sticker_image", {
    coverOnly,
  });
}

/** The secret-bearing Signal link is consumed once by trusted Rust. */
export async function importSignalStickerPack(
  signalLink: string,
): Promise<ImportedStickerDraft> {
  return invokeTauri("import_signal_sticker_pack", { signalLink });
}
