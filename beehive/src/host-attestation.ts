import { schnorr } from '@noble/curves/secp256k1';
import { digest, publicKey } from './protocol.ts';

/** NIP-OA tag as consumed by Buzz 051c3a2 nip_oa.rs. No owner secret is needed to verify/import. */
export type HostAttestation = ['auth', string, string, string];
function clauses(conditions: string): string[] {
  if (conditions.length > 1024) throw Error('Attestation conditions too long');
  if (!conditions) return [];
  const values = conditions.split('&');
  for (const clause of values) {
    const match = /^(kind=|created_at<|created_at>)(0|[1-9][0-9]*)$/.exec(clause);
    if (!match || Number(match[2]) > (match[1] === 'kind=' ? 65535 : 4294967295)) throw Error('Invalid attestation conditions');
  }
  return values;
}
/** Explicit owner-side signing primitive; callers must obtain the existing owner signer
 * interactively. It is not a host bootstrap and must never persist owner secret material.
 */
export function attestHost(ownerSecret: string, host: string, conditions: string): HostAttestation {
  clauses(conditions);
  if (!/^[0-9a-f]{64}$/.test(host) || host === publicKey(ownerSecret)) throw Error('Invalid independent host key');
  return ['auth', publicKey(ownerSecret), conditions, Buffer.from(schnorr.sign(digest(`nostr:agent-auth:${host}:${conditions}`), ownerSecret)).toString('hex')];
}
/** Verify AUTH membership semantics: kind= is deliberately NOT evaluated against 22242,
 * matching verify_auth_tag_for_auth_event. This is not operation/Move authorization.
 */
export function verifyHostAttestation(value: unknown, host: string, owner: string, now: number): HostAttestation {
  if (!Array.isArray(value) || value.length !== 4 || value[0] !== 'auth' || value[1] !== owner || typeof value[2] !== 'string' || typeof value[3] !== 'string' || !/^[0-9a-f]{128}$/.test(value[3]) || !/^[0-9a-f]{64}$/.test(host) || !/^[0-9a-f]{64}$/.test(owner) || owner === host) throw Error('Invalid host attestation');
  for (const clause of clauses(value[2])) {
    if ((clause.startsWith('created_at<') && !(now < Number(clause.slice(11)))) || (clause.startsWith('created_at>') && !(now > Number(clause.slice(11))))) throw Error('Expired/not-yet-valid host attestation');
  }
  if (!schnorr.verify(value[3], digest(`nostr:agent-auth:${host}:${value[2]}`), owner)) throw Error('Invalid owner signature');
  return value as HostAttestation;
}
