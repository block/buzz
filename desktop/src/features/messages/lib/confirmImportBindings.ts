import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_IMPORT_IDENTITY_BINDING,
  KIND_IMPORT_IDENTITY_CLAIM,
} from "@/shared/constants/kinds";

/**
 * Two-party import identity binding join (pure — no React, so it is unit
 * tested directly).
 *
 * Given a mix of owner/admin **attestations** (kind
 * `KIND_IMPORT_IDENTITY_BINDING`, `p` = attested pubkey) and subject
 * **claims** (kind `KIND_IMPORT_IDENTITY_CLAIM`, self-signed, no `p`), returns
 * `<source>:<foreign id>` (e.g. `slack:U060`) → bound pubkey (lowercase hex)
 * ONLY where an attestation and a claim agree: the attested pubkey must equal
 * the claim's author for the same key.
 *
 * This is the trust boundary. An attestation with no matching claim (admin
 * asserting an unconsented mapping) and a claim with no matching attestation
 * (a member asserting an unvouched one) are both dropped — neither can
 * unilaterally attribute imported history to a person.
 */
export function buildConfirmedImportBindings(
  events: RelayEvent[],
): Map<string, string> {
  // Attestations are parameterized-replaceable: newest per key wins, so sort
  // ascending and let later writes overwrite.
  const ordered = [...events].sort(
    (a, b) => a.created_at - b.created_at || a.id.localeCompare(b.id),
  );

  // slack:<id> -> attested pubkey (owner/admin-signed, relay-gated).
  const attested = new Map<string, string>();
  // Self-signed consents, keyed `slack:<id>#<claimant pubkey>`.
  const claimed = new Set<string>();

  for (const event of ordered) {
    const dTag = event.tags.find((t) => t[0] === "d")?.[1];
    if (!dTag) continue;

    if (event.kind === KIND_IMPORT_IDENTITY_BINDING) {
      const pubkey = event.tags.find((t) => t[0] === "p")?.[1];
      if (pubkey?.length !== 64) continue;
      attested.set(dTag, pubkey.toLowerCase());
    } else if (event.kind === KIND_IMPORT_IDENTITY_CLAIM) {
      // The claim's consent is its signature: the author pubkey is the subject.
      if (event.pubkey.length !== 64) continue;
      claimed.add(`${dTag}#${event.pubkey.toLowerCase()}`);
    }
  }

  const confirmed = new Map<string, string>();
  for (const [dTag, pubkey] of attested) {
    if (claimed.has(`${dTag}#${pubkey}`)) confirmed.set(dTag, pubkey);
  }
  return confirmed;
}
