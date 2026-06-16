/**
 * Default public Nostr relays offered as quick-picks when creating a
 * serverless workspace.
 *
 * Originally sourced from the deez mesh client's `DEFAULT_RELAYS`
 * (`deez/crates/mesh-client/src/network/nostr.rs`), but trimmed to the FREE,
 * open relays. `nostr.land` and `nostr.wine` are PAID relays: they answer
 * `auth-required` / `restricted: Pay for access` on reads, which is useless for
 * an open serverless workspace and previously caused an auth-required reconnect
 * storm in the agent. Keep only relays that allow anonymous read/write.
 *
 * `relay.nostr.band` was dropped: it is an indexer/search relay optimized for
 * its REST search API, and in practice never completes a plain client WebSocket
 * handshake from the serverless pool (observed 0/10 connects), so it only ever
 * cost connect timeouts. `relay.primal.net` replaces it — fast, free, and
 * allows anonymous read/write.
 *
 * These are only suggestions — users can type any relay URL. They apply only
 * to serverless workspaces; Sprout-server workspaces use their own relay.
 */
export const DEFAULT_PUBLIC_RELAYS: readonly string[] = [
  "wss://relay.damus.io",
  "wss://nos.lol",
  "wss://relay.primal.net",
] as const;

/** The relays pre-filled (comma-joined) when a user first enables serverless mode. */
export const DEFAULT_SERVERLESS_RELAY = DEFAULT_PUBLIC_RELAYS.join(", ");

/** Parse a comma/whitespace-separated relay string into a clean list. */
export function parseRelayList(value: string): string[] {
  return value
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

/** Whether `relay` is present in the comma-separated `value`. */
export function relayListIncludes(value: string, relay: string): boolean {
  return parseRelayList(value).includes(relay);
}

/** Toggle `relay` in the comma-separated `value`, returning the new value. */
export function toggleRelayInList(value: string, relay: string): string {
  const list = parseRelayList(value);
  const next = list.includes(relay)
    ? list.filter((r) => r !== relay)
    : [...list, relay];
  return next.join(", ");
}

/** Normalize each relay in a comma list to a ws/wss URL; returns comma-joined. */
export function normalizeRelayList(value: string): string {
  return parseRelayList(value)
    .map((r) =>
      r.startsWith("ws://") || r.startsWith("wss://") ? r : `wss://${r}`,
    )
    .join(",");
}
