/**
 * Reachability triage for the relay a `nostrpair://` code asks the *other*
 * device to connect to.
 *
 * A fresh desktop with no community configured falls back to the built-in
 * development relay (`ws://localhost:3000`, see `src-tauri/src/relay.rs`).
 * That address is embedded verbatim in the recovery QR, so a phone scanning it
 * tries to reach *itself* and reports "Could not reach the pairing relay",
 * while the desktop's own connection attempt fails and surfaces as "expired
 * or lost its connection". Neither message points at the actual cause. The
 * mobile client also refuses loopback and private-network relays outright in
 * release builds, so the same triage here mirrors what the phone will reject.
 */

export type PairingRelayReachability =
  | { kind: "reachable"; relayUrl: string; host: string }
  | {
      kind: "local-only";
      relayUrl: string;
      host: string;
      reason: "loopback" | "private-network" | "unspecified";
    }
  | { kind: "unknown" };

const LOOPBACK_HOSTS = new Set(["localhost", "127.0.0.1", "::1", "[::1]"]);
const UNSPECIFIED_HOSTS = new Set(["0.0.0.0", "::", "[::]"]);

/**
 * Extract the `relay` the pairing code instructs the scanning device to join.
 * Returns `null` when the URI does not carry one (or cannot be parsed).
 */
export function pairingRelayFromQrUri(qrUri: string): string | null {
  let parsed: URL;
  try {
    parsed = new URL(qrUri);
  } catch {
    return null;
  }
  const relay = parsed.searchParams.get("relay");
  return relay && relay.trim().length > 0 ? relay.trim() : null;
}

function isPrivateIpv4(host: string): boolean {
  const parts = host.split(".");
  if (parts.length !== 4) return false;
  const octets = parts.map((part) =>
    /^\d{1,3}$/.test(part) ? Number(part) : NaN,
  );
  if (octets.some((octet) => Number.isNaN(octet) || octet > 255)) return false;
  const [a, b] = octets as [number, number, number, number];
  if (a === 10) return true;
  if (a === 172 && b >= 16 && b <= 31) return true;
  if (a === 192 && b === 168) return true;
  if (a === 169 && b === 254) return true;
  return false;
}

/**
 * Classify whether another device on another network could plausibly reach
 * `relayUrl`. Loopback, unspecified, `*.localhost`, and RFC 1918 / link-local
 * addresses are local to the machine (or LAN) that produced the code.
 */
export function classifyPairingRelay(
  relayUrl: string,
): PairingRelayReachability {
  let parsed: URL;
  try {
    parsed = new URL(relayUrl);
  } catch {
    return { kind: "unknown" };
  }
  const host = parsed.hostname.toLowerCase();
  if (!host) return { kind: "unknown" };

  if (LOOPBACK_HOSTS.has(host) || host.endsWith(".localhost")) {
    return { kind: "local-only", relayUrl, host, reason: "loopback" };
  }
  if (UNSPECIFIED_HOSTS.has(host)) {
    return { kind: "local-only", relayUrl, host, reason: "unspecified" };
  }
  if (isPrivateIpv4(host)) {
    return { kind: "local-only", relayUrl, host, reason: "private-network" };
  }
  return { kind: "reachable", relayUrl, host };
}

/** Triage the relay embedded in a `nostrpair://` code in one step. */
export function classifyPairingQrUri(qrUri: string): PairingRelayReachability {
  const relay = pairingRelayFromQrUri(qrUri);
  return relay ? classifyPairingRelay(relay) : { kind: "unknown" };
}

/**
 * Human-readable explanation for a code whose relay another device cannot
 * reach. Names the address so the user can see *what* the desktop is pointed
 * at, and says what to do about it.
 */
export function localOnlyPairingRelayMessage(
  reachability: Extract<PairingRelayReachability, { kind: "local-only" }>,
): string {
  const where =
    reachability.reason === "private-network"
      ? "a private network address"
      : "this computer only";
  return (
    `This desktop isn't connected to a community yet, so the pairing code ` +
    `points at ${reachability.relayUrl} — a relay reachable from ${where}. ` +
    `A phone scanning it can't get there. Join your community first (paste ` +
    `its invite link or relay address), then create a new code.`
  );
}
