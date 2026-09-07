/**
 * Fetch the active relay's NIP-11 `self` pubkey (its own signing key, hex).
 *
 * Buzz signs authoritative kind:30618 repo-state events with this key
 * (`build_ref_state_event` / `relay_keypair`). Clients must scope ref queries
 * to `authors: [self]` so spoofed 30618 events from other pubkeys are ignored.
 *
 * Returns `null` when the relay advertises no `self` or an invalid key.
 * Network / malformed-document failures reject.
 */
import { relayHttpBaseUrl } from "@/shared/lib/relay-url";

const HEX_PUBKEY = /^[0-9a-f]{64}$/;

type Nip11Document = {
  self?: unknown;
};

/** Validate and normalize a NIP-11 `self` value to lowercase hex, or null. */
export function parseRelaySelfPubkey(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const normalized = value.toLowerCase();
  return HEX_PUBKEY.test(normalized) ? normalized : null;
}

/**
 * Trusted authors for a kind:30618 repo-state query.
 *
 * Matches desktop `fetchRepoState`: relay `self` is required (Buzz signs
 * 30618 with the relay keypair). The repo owner is included when known for
 * NIP-34 interop with owner-published state announcements.
 */
export function trustedRepoRefAuthors(
  relaySelf: string,
  ownerPubkey?: string | null,
): string[] {
  return [
    ...new Set(
      [ownerPubkey, relaySelf]
        .map((value) => (typeof value === "string" ? value.toLowerCase() : ""))
        .filter((value) => HEX_PUBKEY.test(value)),
    ),
  ];
}

/** GET the relay NIP-11 document and return its `self` pubkey, or null. */
export async function fetchRelaySelfPubkey(
  signal?: AbortSignal,
): Promise<string | null> {
  const httpUrl = relayHttpBaseUrl();
  // Prefer `/info` (always JSON) — same document as Accept-negotiated `/`.
  const response = await fetch(`${httpUrl.replace(/\/+$/, "")}/info`, {
    headers: { Accept: "application/nostr+json" },
    signal,
  });
  if (!response.ok) {
    return null;
  }
  const doc = (await response.json()) as Nip11Document;
  return parseRelaySelfPubkey(doc.self);
}
