type CurrentProjectionAuthor = Readonly<{
  eventAuthorPubkey: string;
}>;

export function hasCurrentRelayBindingForAuthor(
  projection: CurrentProjectionAuthor | null,
  eventAuthorPubkey: string | null | undefined,
): boolean {
  return (
    projection !== null && projection.eventAuthorPubkey === eventAuthorPubkey
  );
}
