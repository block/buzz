import { existsSync, mkdirSync, rmdirSync } from 'node:fs';
import { join } from 'node:path';
import { initialState, loadSlotState, setupOwner, validateSetup, type Setup } from './host.ts';
import { validateGenesis, type Genesis } from './assignment.ts';
import { object, publicKey } from './protocol.ts';
import { readPrivate, writePrivate } from './storage.ts';
import { credentialReference, createCredential, readCredential, type CredentialBackend, type CredentialReference, systemCredentials } from './credential-store.ts';
import type { SlotEntry } from './slots.ts';

type Harness = Omit<Setup, 'host' | 'ownerSecret' | 'ownerPublic' | 'agentSecret'>;
type Manifest = { version: 3; host: string; ownerPublic: string; setups: Record<string, Harness>; agents: Record<string, { key: CredentialReference; setup: string }> };
const manifestPath = (directory: string) => join(directory, 'setup.json');
const journalPath = (directory: string, agent: string) => join(directory, 'agents', agent, 'journal.json');

function readManifest(directory: string): Manifest {
  const raw = object(readPrivate(manifestPath(directory)));
  if (raw.version !== 3 || Object.hasOwn(raw, 'ownerSecret')) throw Error('Public credential installation required; no automatic plaintext migration');
  const manifest = raw as Manifest;
  setupOwner({ ownerPublic: manifest.ownerPublic });
  const agents = Object.entries(object(manifest.agents));
  if (!agents.length || agents.length > 32) throw Error('Installation requires 1..32 slots');
  for (const harness of Object.values(object(manifest.setups))) {
    const value = object(harness);
    if (['host', 'ownerPublic', 'ownerSecret', 'agentSecret'].some(key => Object.hasOwn(value, key))) throw Error('Binding cannot carry identity');
    validateSetup({ ...value, host: manifest.host, ownerPublic: manifest.ownerPublic });
  }
  for (const [agent, value] of agents) {
    const entry = object(value);
    const ref = object(entry.key);
    if (Object.hasOwn(entry, 'secret') || ref.service !== 'beehive' || ref.role !== 'agent' || ref.publicKey !== agent) throw Error('Invalid agent credential reference');
    credentialReference('agent', agent);
    if (typeof entry.setup !== 'string' || !Object.hasOwn(manifest.setups, entry.setup)) throw Error('Invalid slot binding');
    // Validate preserved authority before any credential mutation or hydration.
    loadSlotState({ ...manifest.setups[entry.setup]!, host: manifest.host, ownerPublic: manifest.ownerPublic }, journalPath(directory, agent), agent);
  }
  return manifest;
}

/** Read OS entries anew on every load. Missing means public-only, never regeneration;
 * locked/denied/unavailable errors propagate, rather than masquerading as absence. */
export function credentialSlots(directory: string, backend: CredentialBackend = systemCredentials): SlotEntry[] {
  const manifest = readManifest(directory);
  return Object.entries(manifest.agents).map(([agent, entry]) => {
    const present = backend.read(entry.key) !== null;
    const secret = present ? readCredential(entry.key, backend) : undefined;
    const bindings = Object.fromEntries(Object.entries(manifest.setups).map(([id, harness]) => [id, validateSetup({ ...harness, host: manifest.host, ownerPublic: manifest.ownerPublic, ...(secret ? { agentSecret: secret } : {}) })]));
    return { agent, path: journalPath(directory, agent), setupId: entry.setup, setup: bindings[entry.setup]!, bindings, keyPresent: present };
  });
}

function locked<T>(directory: string, action: () => T): T {
  const lock = join(directory, 'host.lock');
  mkdirSync(lock, { mode: 0o700 });
  try { return action(); } finally { rmdirSync(lock); }
}

/** Fresh explicit provisioning only. The inert journal precedes credential creation;
 * read-back verification precedes public activation. Partial attempts refuse reset.
 * This is not a transaction across the filesystem and OS store. */
export function provisionCredentialSlot(directory: string, setup: Setup, secret: string, root: Genesis, backend: CredentialBackend = systemCredentials): void {
  if (setup.ownerSecret !== undefined || setup.agentSecret !== undefined) throw Error('Provisioning accepts owner public identity and separately supplied agent key only');
  validateSetup(setup);
  const ownerPublic = setupOwner(setup), agent = publicKey(secret), genesis = validateGenesis(root);
  if (genesis.owner !== ownerPublic || genesis.agent !== agent) throw Error('Genesis ownership mismatch');
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  locked(directory, () => {
    if (existsSync(manifestPath(directory)) || existsSync(journalPath(directory, agent))) throw Error('Installation or partial provision exists; refusing identity reset');
    const { host, ownerSecret: _owner, ownerPublic: _public, agentSecret: _agent, ...harness } = setup;
    writePrivate(journalPath(directory, agent), initialState({ ...setup, agentSecret: secret }, genesis), true);
    const key = createCredential('agent', secret, backend);
    const manifest: Manifest = { version: 3, host, ownerPublic, setups: { default: harness }, agents: { [agent]: { key, setup: 'default' } } };
    writePrivate(manifestPath(directory), manifest, true);
  });
}

/** Preserve the public reference and every journal byte even after deliberate deletion.
 * A retry after successful deletion verifies absence; it never recreates a credential. */
export function removeCredentialSlotKey(directory: string, agent: string, backend: CredentialBackend = systemCredentials): void {
  locked(directory, () => {
    const manifest = readManifest(directory), entry = manifest.agents[agent];
    if (!entry) throw Error('Unknown retained public slot');
    const state = loadSlotState({ ...manifest.setups[entry.setup]!, host: manifest.host, ownerPublic: manifest.ownerPublic }, journalPath(directory, agent), agent);
    if (state.phase !== 'stopped' || state.actual !== null) throw Error('Key removal requires a stopped slot with no actual run');
    if (backend.read(entry.key) !== null) { readCredential(entry.key, backend); backend.remove(entry.key); }
    if (backend.read(entry.key) !== null) throw Error('Credential removal not verified');
  });
}

/** Explicit restoration of the same retained key only; possession grants no assignment. */
export function importCredentialSlotKey(directory: string, agent: string, secret: string, backend: CredentialBackend = systemCredentials): void {
  if (publicKey(secret) !== agent) throw Error('Imported key does not match the selected public identity');
  locked(directory, () => {
    const manifest = readManifest(directory), entry = manifest.agents[agent];
    if (!entry) throw Error('Unknown retained public slot');
    const state = loadSlotState({ ...manifest.setups[entry.setup]!, host: manifest.host, ownerPublic: manifest.ownerPublic }, journalPath(directory, agent), agent);
    if (state.phase !== 'stopped' || state.actual !== null) throw Error('Key import requires a stopped slot with no actual run');
    createCredential('agent', secret, backend);
  });
}
