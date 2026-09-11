import { bootstrapHostIdentity } from '../src/host-identity.ts';
import { isolatedFileCredentials } from './isolated-file-credentials.ts';
import { createCredential, credentialReference, readCredential } from '../src/credential-store.ts';
import { publicKey } from '../src/protocol.ts';
import type { managementClient } from '../src/intents.ts';

/** Installed-launcher smoke ONLY: explicit synthetic credential file and zero transport. */
export async function fixtureCredential(input: any, signal: AbortSignal) {
  signal.throwIfAborted();
  const backend = isolatedFileCredentials(process.env.BEEHIVE_TEST_CREDENTIAL_FILE!);
  if (input.action === 'configure') {
    bootstrapHostIdentity(input.directory,input.label,input.owner,input.relay,backend);
    return { ok: true };
  }
  if (input.action !== 'signin') throw Error('Fixture supports configuration and owner sign-in only');
  if (input.secret) {
    if (publicKey(input.secret) !== input.owner) throw Error('Wrong fixture owner');
    createCredential('owner',input.secret,backend);
  }
  const reference = credentialReference('owner',input.owner);
  return { ok: true, secret: backend.read(reference) === null ? null : readCredential(reference,backend) };
}
/** Never opens a socket, even if a supplied fixture URL is accidentally real. */
export const disconnectedTransport = (() => ({ ready: Promise.resolve(), connected: false, status: () => [], close() {}, reconcile() {}, submit() { throw Error('Explicit fixture transport is disconnected'); } })) as unknown as typeof managementClient;
