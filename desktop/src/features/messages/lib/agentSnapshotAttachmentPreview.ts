import { previewAgentSnapshotImport } from "@/shared/api/tauriPersonas";
import type { BlobDescriptor } from "@/shared/api/tauri";
import { fetchSnapshotBytes } from "@/shared/api/tauriMedia";

type PreviewDependencies = {
  fetchBytes?: typeof fetchSnapshotBytes;
  previewImport?: typeof previewAgentSnapshotImport;
};

export function isAgentSnapshotPngFilename(
  filename: string | null | undefined,
): boolean {
  return filename?.toLowerCase().endsWith(".agent.png") ?? false;
}

/**
 * Add a composer-only avatar to an uploaded `.agent.png` descriptor.
 *
 * The relay attachment remains the complete trading card. We decode the
 * already-validated snapshot only to recover its source avatar for the compact
 * composer chip, and deliberately keep that URL out of NIP-92 imeta fields.
 * Any legacy/malformed snapshot simply retains the full-card fallback.
 */
export async function withAgentSnapshotAvatarPreview(
  descriptor: BlobDescriptor,
  fileBytes?: Uint8Array | number[],
  dependencies: PreviewDependencies = {},
): Promise<BlobDescriptor> {
  const filename = descriptor.filename;
  if (
    descriptor.snapshotAvatarUrl ||
    !isAgentSnapshotPngFilename(filename) ||
    !filename
  ) {
    return descriptor;
  }

  try {
    const bytes = fileBytes
      ? Array.from(fileBytes)
      : await (dependencies.fetchBytes ?? fetchSnapshotBytes)({
          url: descriptor.url,
          filename,
          expectedSha256: descriptor.sha256,
          expectedSize: descriptor.size,
        });
    const preview = await (
      dependencies.previewImport ?? previewAgentSnapshotImport
    )(bytes, filename);
    const avatarUrl =
      preview.sourceAvatarUrl?.trim() || preview.avatarUrl?.trim();
    return avatarUrl
      ? { ...descriptor, snapshotAvatarUrl: avatarUrl }
      : descriptor;
  } catch {
    return descriptor;
  }
}
