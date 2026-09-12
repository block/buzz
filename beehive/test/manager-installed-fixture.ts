import { registerAgent, agentNsec, addOpenAI } from '../src/settings-credentials.ts';
import type { CredentialReference } from '../src/credential-store.ts';
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
  if (input.action === 'register-agent') { registerAgent(input.directory,agentNsec(input.secret),backend,input.profile); return { ok: true }; }
  if (input.action === 'add-openai') { addOpenAI(input.directory,input.name,input.secret,{ read: r => backend.read(r as unknown as CredentialReference), create: (r,s) => backend.create(r as unknown as CredentialReference,s) }); return { ok: true }; }
  if (input.action === 'models') return { ok: true, models: ['fixture-model'] };
  if (input.action !== 'signin') throw Error('Unsupported installed fixture action');
  if (input.secret) {
    if (publicKey(input.secret) !== input.owner) throw Error('Wrong fixture owner');
    createCredential('owner',input.secret,backend);
  }
  const reference = credentialReference('owner',input.owner);
  return { ok: true, secret: backend.read(reference) === null ? null : readCredential(reference,backend) };
}
/** Never opens a socket, even if a supplied fixture URL is accidentally real. */
export const disconnectedTransport = (() => ({ ready: Promise.resolve(), connected: false, status: () => [], close() {}, reconcile() {}, submit() { throw Error('Explicit fixture transport is disconnected'); } })) as unknown as typeof managementClient;
