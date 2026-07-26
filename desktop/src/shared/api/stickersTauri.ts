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

/**
 * Import a Sonar/Nostr pack already published on public relays. The link is
 * public metadata (no secrets); the pack's asset URLs are kept as-is and the
 * importer republishes the pack under their own key on publish.
 */
export async function importNostrStickerPack(
  link: string,
): Promise<ImportedStickerDraft> {
  return invokeTauri("import_nostr_sticker_pack", { link });
}
