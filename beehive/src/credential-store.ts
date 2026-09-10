import { publicKey } from './protocol.ts';

/** Beehive namespace only. Never query Buzz Desktop's service or legacy entries. */
export const credentialService = 'beehive';
export type CredentialReference = { service: 'beehive'; role: 'host' | 'agent' | 'owner'; publicKey: string };
/** Native implementations must serialize cross-process mutations and read back
 * from the OS, not a cache. Missing differs from locked/denied/unavailable (throw).
 * Tests inject an explicitly isolated backend; they do not validate OS storage.
 */
export interface CredentialBackend {
  read(reference: CredentialReference): string | null;
  create(reference: CredentialReference, secret: string): void;
  remove(reference: CredentialReference): void;
}
/** No vetted TS native bridge is present. No prompting or plaintext fallback. */
export const systemCredentials: CredentialBackend = {
  read() { throw unavailable(); }, create() { throw unavailable(); }, remove() { throw unavailable(); },
};
function unavailable() { return Error('Beehive OS credential store unavailable: a reviewed native Keychain/Secret Service/Credential Manager bridge is required. No plaintext fallback; existing keys are not migrated or regenerated.'); }
/** Public references never contain a private identity key. */
export function credentialReference(role: CredentialReference['role'], key: string): CredentialReference {
  if (!['host', 'agent', 'owner'].includes(role) || !/^[0-9a-f]{64}$/.test(key)) throw Error('Invalid credential reference');
  return { service: credentialService, role, publicKey: key };
}
/** Missing/removed keys must be explicitly imported, never silently recreated. */
export function readCredential(reference: CredentialReference, backend: CredentialBackend = systemCredentials): string {
  if (reference.service !== credentialService) throw Error('Wrong credential namespace');
  credentialReference(reference.role, reference.publicKey);
  const secret = backend.read(reference);
  if (secret === null) throw Error('Local identity key missing or intentionally removed; explicitly import the matching key');
  if (publicKey(secret) !== reference.publicKey) throw Error('Credential identity mismatch');
  return secret;
}
/** Explicit creation/import only; verify the backend round trip before public state. */
export function createCredential(role: CredentialReference['role'], secret: string, backend: CredentialBackend = systemCredentials): CredentialReference {
  const reference = credentialReference(role, publicKey(secret));
  if (backend.read(reference) !== null) throw Error('Credential already exists; refusing replacement');
  backend.create(reference, secret);
  if (readCredential(reference, backend) !== secret) throw Error('Credential verification failed');
  return reference;
}
/** Verified absence precedes changing public-only metadata. Failures propagate. */
export function removeCredential(reference: CredentialReference, backend: CredentialBackend = systemCredentials): void {
  readCredential(reference, backend);
  backend.remove(reference);
  if (backend.read(reference) !== null) throw Error('Credential removal not verified');
}
