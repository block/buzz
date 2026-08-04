import type { UserSearchResult } from "@/shared/api/types";

export function formatSearchUserDisplayName(user: UserSearchResult) {
  return user.displayName?.trim() || user.nip05Handle?.trim() || null;
}

export function formatSearchUserSecondaryLabel(user: UserSearchResult) {
  const displayName = user.displayName?.trim();
  const nip05Handle = user.nip05Handle?.trim();
  if (displayName && nip05Handle) {
    return nip05Handle;
  }
  return null;
}
