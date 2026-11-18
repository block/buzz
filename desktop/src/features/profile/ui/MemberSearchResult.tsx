/**
 * The member picker's shared parts: one result row, its label, and the
 * candidate list built from a pluggable search source plus a pasted
 * npub/hex key. Add Member feeds it `useUserSearchQuery`; the Admin Console
 * feeds it the relay's admin member search.
 */

import type { UserSearchResult } from "@/shared/api/types";
import { parsePubkeyInput } from "@/shared/lib/nostrUtils";
import { truncateNpub } from "@/shared/lib/pubkey";
import { ProfileAvatar } from "./ProfileAvatar";

export function memberSearchLabel(user: UserSearchResult): string {
  return (
    user.displayName?.trim() ||
    user.nip05Handle?.trim() ||
    truncateNpub(user.pubkey)
  );
}

/** A pasted key first (unless search already found it), then search hits. */
export function useMemberCandidates(
  query: string,
  source: { data?: UserSearchResult[] },
): UserSearchResult[] {
  const parsed = parsePubkeyInput(query);
  const found = source.data ?? [];
  if (parsed === null || found.some((u) => u.pubkey === parsed)) return found;
  return [
    {
      pubkey: parsed,
      displayName: null,
      avatarUrl: null,
      nip05Handle: null,
      ownerPubkey: null,
      isAgent: false,
    },
    ...found,
  ];
}

export function MemberSearchResult({
  onSelect,
  showKey = false,
  testId,
  user,
}: {
  onSelect: () => void;
  /** Show each named result's short npub, so same-name results differ. */
  showKey?: boolean;
  testId: string;
  user: UserSearchResult;
}) {
  const name = memberSearchLabel(user);
  const isDirectPubkey = user.displayName === null && user.nip05Handle === null;
  return (
    <button
      className="flex min-h-11 w-full items-center gap-3 px-3 py-2 text-left transition-colors hover:bg-muted/50 focus-visible:bg-muted/50 focus-visible:outline-hidden"
      data-testid={testId}
      onClick={onSelect}
      role="option"
      type="button"
    >
      <ProfileAvatar
        avatarUrl={user.avatarUrl}
        className="h-8 w-8 text-xs shadow-none"
        iconClassName="h-4 w-4"
        label={name}
        shape={user.isAgent ? "squircle" : "circle"}
      />
      <span className="min-w-0 flex-1 truncate text-sm font-medium">
        {name}
      </span>
      {isDirectPubkey || showKey ? (
        <span className="shrink-0 text-xs text-muted-foreground">
          {isDirectPubkey ? "public key" : truncateNpub(user.pubkey)}
        </span>
      ) : null}
    </button>
  );
}
