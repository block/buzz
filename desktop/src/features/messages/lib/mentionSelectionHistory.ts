import { normalizePubkey } from "@/shared/lib/pubkey";

// Community lifetime is owned by resetCommunityState. Keys additionally isolate
// identities/channels; this is selection intent, never authorization or pins.
const history = new Map<string, string[]>();
const scopeKey = (viewer: string, channel: string) =>
  `${normalizePubkey(viewer)}:${channel}`;

/** Clear selection intent when the community changes. */
export function resetMentionSelectionHistory() {
  history.clear();
}

/** Read recent successful insertions for this viewer and channel only. */
export function getMentionSelectionHistory(
  viewer: string | null,
  channel: string | null,
): readonly string[] {
  return viewer && channel
    ? (history.get(scopeKey(viewer, channel)) ?? [])
    : [];
}

/** Record a successful exact-key insertion without creating permission or a pin. */
export function rememberMentionSelection(
  viewer: string | null,
  channel: string | null,
  pubkey: string,
) {
  if (!viewer || !channel) return;
  const scope = scopeKey(viewer, channel);
  const key = normalizePubkey(pubkey);
  const previous = history.get(scope) ?? [];
  history.delete(scope);
  history.set(
    scope,
    [key, ...previous.filter((item) => item !== key)].slice(0, 50),
  );
  while (history.size > 100) {
    const oldest = history.keys().next().value;
    if (oldest === undefined) break;
    history.delete(oldest);
  }
}
