import type { UserProfileSummary } from "@/shared/api/types";

/**
 * Resolve a sender's human display label from a profile summary, mirroring
 * the sidebar's precedence (`resolveUserLabel`: displayName, then NIP-05
 * handle) — but returning `null` instead of a truncated pubkey when no real
 * name is known. Notification titles fall back to neutral copy ("Reply in
 * #channel", "Direct message"), never a hex fragment.
 */
export function senderNameFromSummary(
  summary: UserProfileSummary | null | undefined,
): string | null {
  const displayName = summary?.displayName?.trim();
  if (displayName) {
    return displayName;
  }

  const nip05Handle = summary?.nip05Handle?.trim();
  if (nip05Handle) {
    return nip05Handle;
  }

  return null;
}
