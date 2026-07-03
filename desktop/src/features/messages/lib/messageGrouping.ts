type MessageAuthorCandidate = {
  pubkey?: string | null;
};

export function hasSameMessageAuthor(
  previous: MessageAuthorCandidate | null | undefined,
  current: MessageAuthorCandidate | null | undefined,
) {
  const previousPubkey = previous?.pubkey?.trim().toLowerCase();
  const currentPubkey = current?.pubkey?.trim().toLowerCase();

  return Boolean(
    previousPubkey && currentPubkey && previousPubkey === currentPubkey,
  );
}
