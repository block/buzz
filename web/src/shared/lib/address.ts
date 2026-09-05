/**
 * Relay address normalization — the single input of join-by-address.
 *
 * A stranger has ONE line of text, copied off a poster or a friend's phone:
 *   wss://relay.skaists.dev   relay.skaists.dev   https://skaists.buzz
 * All three must land on the same community. The address is normalized to an
 * origin (for HTTP pairing material) and a WebSocket URL (for the live room).
 * Anything that does not parse as a host is refused — no guessing, no search.
 */

export type RelayAddress = {
  /** HTTP origin, no trailing slash — where pairing material is served. */
  origin: string;
  /** WebSocket URL — the room connection (the road the user was given). */
  wsUrl: string;
  /** Bare host[:port] — the community's identity in the UI. */
  host: string;
  /** The community's canonical WS origin (NIP-42 `relay` tag) when known. */
  canonicalRelayUrl?: string;
};

export function normalizeRelayAddress(input: string): RelayAddress | null {
  const trimmed = input.trim();
  if (!trimmed || trimmed.length > 253) return null;

  let candidate = trimmed;
  let scheme: "https" | "http" = "https";
  if (candidate.startsWith("wss://")) {
    candidate = candidate.slice(6);
  } else if (candidate.startsWith("ws://")) {
    candidate = candidate.slice(5);
    scheme = "http";
  } else if (candidate.startsWith("https://")) {
    candidate = candidate.slice(8);
  } else if (candidate.startsWith("http://")) {
    candidate = candidate.slice(7);
    scheme = "http";
  }

  // Strip any path/query/fragment — an address is a host, not a document.
  const hostPart = candidate.split("/")[0].split("?")[0].split("#")[0];
  if (!hostPart) return null;

  // Host[:port], letters/digits/dots/dashes/colons — IPv6 stays bracketed.
  const hostRe = /^\[?[a-zA-Z0-9.-]+\]?(:[0-9]{1,5})?$/;
  if (!hostRe.test(hostPart)) return null;
  const port = hostPart.match(/:([0-9]{1,5})$/)?.[1];
  if (port !== undefined && Number(port) > 65535) return null;

  const secure = scheme === "https";
  return {
    origin: `${scheme}://${hostPart}`,
    wsUrl: `${secure ? "wss" : "ws"}://${hostPart}`,
    host: hostPart,
  };
}
