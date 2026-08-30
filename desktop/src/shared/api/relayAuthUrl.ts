export type RelayTransport = "lan" | "public" | null;

/**
 * Build the NIP-42 relay tag for the transport that actually reached the relay.
 *
 * LAN fast-path sockets keep the canonical host for tenant routing, but dial
 * the relay directly over a plain WebSocket. The relay therefore sees `ws`
 * while a public reverse proxy presents `wss`.
 */
export function relayAuthUrlForTransport(
  canonicalRelayUrl: string,
  transport: RelayTransport,
): string {
  if (transport !== "lan") return canonicalRelayUrl;

  const parsed = new URL(canonicalRelayUrl);
  if (parsed.protocol === "wss:") parsed.protocol = "ws:";
  if (parsed.protocol !== "ws:") {
    throw new Error("Relay authentication URL must use ws:// or wss://.");
  }

  return parsed.href.replace(/\/$/, "");
}
