import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";

/**
 * Environment / machine label for a host lineage pubkey from kind:10100 `host`.
 *
 * Hosts are not person profiles — do not route through `resolveUserLabel` or
 * kind:0. Prefer an explicit known-hosts map (house config later); otherwise
 * truncate the pubkey.
 */
export function resolveHostLabel(input: {
  hostPubkey: string;
  knownHosts?: Readonly<Record<string, string>>;
}): string {
  const hostPubkey = normalizePubkey(input.hostPubkey);
  if (hostPubkey.length === 0) {
    return "host";
  }

  const known = input.knownHosts?.[hostPubkey]?.trim();
  if (known) {
    return known;
  }

  // knownHosts may be keyed with mixed case from callers
  if (input.knownHosts) {
    for (const [key, label] of Object.entries(input.knownHosts)) {
      if (normalizePubkey(key) === hostPubkey) {
        const trimmed = label?.trim();
        if (trimmed) return trimmed;
      }
    }
  }

  return truncatePubkey(hostPubkey);
}
