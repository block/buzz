import type { ChatMetadata } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

/** Chat metadata authored by someone else — a chat shared with us. */
export function isSharedChatMetadata(
  metadata: ChatMetadata | null | undefined,
  identityPubkey: string | null | undefined,
) {
  if (!metadata?.authorPubkey || !identityPubkey) {
    return false;
  }
  return (
    normalizePubkey(metadata.authorPubkey) !== normalizePubkey(identityPubkey)
  );
}
