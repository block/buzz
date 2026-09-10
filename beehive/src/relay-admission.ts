import { randomUUID } from 'node:crypto';
import { finalizeEvent } from 'nostr-tools/pure';
import { publicKey } from './protocol.ts';
import { verifyDirectMembershipEvidence, type HostAuthorizationSigner } from './direct-membership.ts';
/** Admission proof supplied by a reviewed integration, never loaded from user JSON.
 * Owner-approved route: ordinary direct relay membership, without NIP-OA.
 * Registration alone or successful open-relay AUTH is not membership evidence.
 */
export type ScopedRelayAdmission = {
  admit(scope: { relay: string; publicKey: string; transport: 'nip42-nip59'; ownerDelegation: false }): void;
};
/** No production admission profile is established. Do not dial or send NIP-OA. */
export const productionAdmission: ScopedRelayAdmission = {
  admit() { throw Error('Host relay admission pending: verified ordinary direct membership required (relay admin: buzz-admin add-member --pubkey <host-public-key> --role member); NIP-OA prohibited'); },
};

/** Infrastructure or owner signs only its own fresh HTTP authentication; no OA.
 * Match Buzz CLI's nonce tag: same-second checks/claim retries must have distinct
 * event IDs for the community-scoped NIP-98 replay guard. */
export function membershipSigner(secret: string): HostAuthorizationSigner {
  return { hostPublicKey: publicKey(secret), async signHttpAuthentication(request) {
    return JSON.stringify(finalizeEvent({ kind: 27235, created_at: Math.floor(Date.now() / 1000), content: '',
      tags: [['u', request.url], ['method', request.method], ['payload', request.payloadSha256Hex], ['nonce', randomUUID()]] }, Buffer.from(secret, 'hex')));
  } };
}
/** One fresh live row check authorizes ONE immediate connection. Reopen the command
 * for another check after loss; a saved claim/roster/ACK is never an admission token. */
export async function verifyProductionAdmission(relay: string, secret: string, signal?: AbortSignal): Promise<ScopedRelayAdmission> {
  const signer = membershipSigner(secret);
  const evidence = await verifyDirectMembershipEvidence({ relay, signer, signal });
  signal?.throwIfAborted();
  if (evidence.status !== 'live-verified-member') throw Error(`Direct membership not verified (${evidence.status}); require membership-enforced relay and a fresh live row check`);
  const deadline = Date.now() + 10_000;
  let consumed = false;
  return { admit(scope) {
    if (consumed || Date.now() > deadline) throw Error('Fresh membership required; close and manually reopen command');
    if (scope.relay !== relay || scope.publicKey !== signer.hostPublicKey || scope.transport !== 'nip42-nip59' || scope.ownerDelegation !== false) throw Error('Wrong direct membership scope');
    consumed = true;
  } };
}
