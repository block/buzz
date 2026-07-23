import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";

const AVATAR_MEDIA_RE =
  /^https?:\/\/[^/]+\/media\/[\da-f]{64}(?:\.thumb)?\.(?:jpg|png|gif|webp)(?:\?.*)?$/iu;

const INLINE_AVATAR_RE = /^(?:blob:|data:image\/)/iu;

function canonicalOrigin(url: string): string | null {
  try {
    return new URL(url).origin;
  } catch {
    return null;
  }
}

export function isInlineAvatarUrl(url: string | null | undefined): boolean {
  return INLINE_AVATAR_RE.test(url?.trim() ?? "");
}

export function isRelayHostedAvatarUrl(
  url: string | null | undefined,
  relayOrigin: string | null,
): boolean {
  const trimmed = url?.trim();
  if (!trimmed || !relayOrigin || !AVATAR_MEDIA_RE.test(trimmed)) return false;
  return canonicalOrigin(trimmed) === relayOrigin;
}

export function resolveAvatarImageSrc(
  url: string | null | undefined,
  relayOrigin: string | null,
): string | null {
  const trimmed = url?.trim();
  if (!trimmed) return null;
  if (isInlineAvatarUrl(trimmed)) return trimmed;
  if (!isRelayHostedAvatarUrl(trimmed, relayOrigin)) return null;
  return rewriteRelayUrl(trimmed);
}
