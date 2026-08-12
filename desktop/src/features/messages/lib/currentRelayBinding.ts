type CurrentProjectionAuthor = Readonly<{
  eventAuthorPubkey: string;
  freshUntil: number;
}>;

/** Return true only for an exact author match before the projection expires. */
export function hasCurrentRelayBindingForAuthor(
  projection: CurrentProjectionAuthor | null,
  eventAuthorPubkey: string | null | undefined,
  nowSeconds = Date.now() / 1_000,
): boolean {
  return (
    projection !== null &&
    Number.isFinite(nowSeconds) &&
    nowSeconds < projection.freshUntil &&
    projection.eventAuthorPubkey === eventAuthorPubkey
  );
}
