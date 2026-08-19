/**
 * Deriving a default clone URL for a NIP-34 repo announcement that omits an
 * explicit `clone` tag.
 *
 * Buzz relays serve their own git repositories at a canonical path —
 * `<relay-origin>/git/<owner-hex>/<repo-id>` — which is exactly the shape the
 * Rust `validate_clone_url` gate enforces. When an announcement carries no
 * `clone` tag (e.g. it was created via `buzz repos create` without `--clone`),
 * the desktop would otherwise have no URL to fetch from, so the project detail
 * view comes up empty. Synthesizing the canonical relay-hosted URL lets those
 * repositories load while still deferring to any explicit clone URLs.
 */

import { parsePubkeyInput } from "@/shared/lib/nostrUtils";

/**
 * Builds the canonical relay-hosted clone URL for a repository, or `null` when
 * the inputs cannot produce a valid URL (unresolved relay origin, missing owner
 * pubkey, or missing repo id). Fails closed rather than emitting a broken URL.
 *
 * `relayOrigin` is expected to be a bare origin (scheme + host, e.g.
 * `https://relay.example`); a trailing slash is tolerated.
 */
export function deriveRelayCloneUrl(
  relayOrigin: string | null | undefined,
  owner: string,
  dtag: string,
): string | null {
  if (!relayOrigin || !owner || !dtag) return null;
  // The Git smart-HTTP transport is an intentional protocol exception: its
  // path segment is the lowercase NIP-01 key, even when callers supply npub.
  const ownerHex = parsePubkeyInput(owner);
  if (!ownerHex) return null;
  const origin = relayOrigin.replace(/\/+$/, "");
  return `${origin}/git/${ownerHex}/${dtag}`;
}

/**
 * Returns the effective clone URLs for a project: the explicitly advertised
 * ones when present, otherwise a single-element list holding the derived
 * relay-hosted default (or an empty list when no default can be derived).
 *
 * Explicit `clone` tags always win — NIP-34 permits pointing `clone` at an
 * external host (e.g. GitHub), which must not be overridden.
 */
export function effectiveCloneUrls(
  cloneUrls: string[],
  relayOrigin: string | null | undefined,
  owner: string,
  dtag: string,
): string[] {
  if (cloneUrls.length > 0) return cloneUrls;
  const derived = deriveRelayCloneUrl(relayOrigin, owner, dtag);
  return derived ? [derived] : [];
}
