import { type BlobDescriptor, uploadMediaBytes } from "@/shared/api/tauri";

type UploadMediaBytes = (
  data: number[],
  filename?: string,
) => Promise<BlobDescriptor>;

export async function resolveManagedAgentAvatarUrl(
  avatarUrl: string | null | undefined,
  upload: UploadMediaBytes = uploadMediaBytes,
  fallbackAvatarUrl?: string | null,
): Promise<string | undefined> {
  const resolvedAvatarUrl = avatarUrl?.trim() || undefined;
  if (!resolvedAvatarUrl?.startsWith("data:image/")) {
    return resolvedAvatarUrl;
  }

  try {
    const [, b64] = resolvedAvatarUrl.split(",", 2);
    if (!b64) {
      throw new Error("empty data URI payload");
    }
    const bytes = Array.from(atob(b64), (char) => char.charCodeAt(0));
    const blob = await upload(bytes);
    return blob.url;
  } catch (err) {
    console.warn("Avatar upload failed, proceeding without avatar:", err);
    return safeFallbackAvatarUrl(fallbackAvatarUrl);
  }
}

function safeFallbackAvatarUrl(avatarUrl: string | null | undefined) {
  const trimmed = avatarUrl?.trim() || undefined;
  return trimmed?.startsWith("data:image/") ? undefined : trimmed;
}
