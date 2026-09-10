import { existsSync, mkdirSync, readdirSync, rmdirSync } from 'node:fs';
import { join } from 'node:path';
import { initialState, loadSlotState, setupOwner, validateSetup, bindingFingerprint, setupModels, type Setup } from './host.ts';
import { validateGenesis, type Genesis } from './assignment.ts';
import { object, publicKey, text } from './protocol.ts';
import { semanticHash } from './handoff.ts';
import { readPrivate, writePrivate } from './storage.ts';
import { credentialReference, createCredential, readCredential, type CredentialBackend, type CredentialReference, systemCredentials } from './credential-store.ts';
import type { ConversationSetup } from './conversation.ts';
import type { SlotEntry } from './slots.ts';

type Harness = Omit<Setup, 'host' | 'ownerSecret' | 'ownerPublic' | 'agentSecret'>;
type Manifest = { retiredBindings?: Record<string, string>; conversation?: ConversationSetup; version: 3; host: string; ownerPublic: string; setups: Record<string, Harness>; agents: Record<string, { key: CredentialReference; setup: string }> };
const manifestPath = (directory: string) => join(directory, 'setup.json');
const journalPath = (directory: string, agent: string) => join(directory, 'agents', agent, 'journal.json');

/** Validate only public installation metadata and retained journals, never credentials. */
export function readCredentialManifest(directory: string): Manifest {
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
  if (manifest.retiredBindings !== undefined) {
    for (const [id, fingerprint] of Object.entries(object(manifest.retiredBindings))) {
      if (!Object.hasOwn(manifest.setups, id) || fingerprint !== semanticHash(manifest.setups[id])) throw Error('Invalid retired binding definition');
    }
  }
  const common = manifest.conversation ?? manifest.setups.default?.conversation;
  for (const harness of Object.values(manifest.setups)) {
    if (harness.conversation && semanticHash(harness.conversation) !== semanticHash(common ?? null)) throw Error('Binding cannot change conversation authority');
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
  const manifest = readCredentialManifest(directory);
  return Object.entries(manifest.agents).map(([agent, entry]) => {
    const present = backend.read(entry.key) !== null;
    const secret = present ? readCredential(entry.key, backend) : undefined;
    const bindings = Object.fromEntries(Object.entries(manifest.setups).map(([id, harness]) => [id, validateSetup({ ...harness, host: manifest.host, ownerPublic: manifest.ownerPublic, ...(secret ? { agentSecret: secret } : {}) })]));
    return { retiredBindings: manifest.retiredBindings, agent, path: journalPath(directory, agent), setupId: entry.setup, setup: bindings[entry.setup]!, bindings, keyPresent: present };
  });
}

/** Cancellation-fenced live hydration. Public input is rechecked after each read;
 * no secret result can hydrate a changed manifest or bypass a removed key. */
export async function credentialSlotsAsync(directory: string, backend: CredentialBackend, signal?: AbortSignal): Promise<SlotEntry[]> {
  signal?.throwIfAborted();
  const manifest = readCredentialManifest(directory);
  const snapshot = JSON.stringify(manifest);
  const secrets = new Map<string, string | null>();
  for (const entry of Object.values(manifest.agents)) {
    signal?.throwIfAborted();
    const secret = backend.readAsync ? await backend.readAsync(entry.key, signal) : backend.read(entry.key);
    signal?.throwIfAborted();
    if (JSON.stringify(readCredentialManifest(directory)) !== snapshot) throw Error('Credential manifest changed during read');
    if (secret !== null && publicKey(secret) !== entry.key.publicKey) throw Error('Credential identity mismatch');
    secrets.set(entry.key.publicKey, secret);
  }
  // Reuse the normal validation/materialization without another native read.
  return credentialSlots(directory, { read: ref => {
    if (!secrets.has(ref.publicKey)) throw Error('Credential snapshot changed');
    return secrets.get(ref.publicKey)!;
  }, create() { throw Error('Read only'); }, remove() { throw Error('Read only'); } });
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
    const manifest = readCredentialManifest(directory), entry = manifest.agents[agent];
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
    const manifest = readCredentialManifest(directory), entry = manifest.agents[agent];
    if (!entry) throw Error('Unknown retained public slot');
    const state = loadSlotState({ ...manifest.setups[entry.setup]!, host: manifest.host, ownerPublic: manifest.ownerPublic }, journalPath(directory, agent), agent);
    if (state.phase !== 'stopped' || state.actual !== null) throw Error('Key import requires a stopped slot with no actual run');
    createCredential('agent', secret, backend);
  });
}

/** Explicit same-identity recovery of an interrupted FIRST provision only. The
 * complete inert genesis must match; active/public-only slots never enter here.
 * No auto-adoption, key generation, overwriting or historical journal reset. */
export function reconcileCredentialProvision(directory: string, setup: Setup, secret: string, root: Genesis, backend: CredentialBackend = systemCredentials): void {
  if (setup.ownerSecret !== undefined || setup.agentSecret !== undefined) throw Error('Public provisioning setup required');
  validateSetup(setup);
  const agent = publicKey(secret), genesis = validateGenesis(root), ownerPublic = setupOwner(setup);
  if (genesis.owner !== ownerPublic || genesis.agent !== agent) throw Error('Genesis ownership mismatch');
  locked(directory, () => {
    if (existsSync(manifestPath(directory))) throw Error('Active installation exists; use explicit retained-key import, never provision recovery');
    const expected = initialState({ ...setup, agentSecret: secret }, genesis);
    const actual = readPrivate(journalPath(directory, agent));
    if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error('Partial journal differs from exact inert genesis; refusing reset');
    const key = credentialReference('agent', agent);
    const existing = backend.read(key);
    if (existing === null) createCredential('agent', secret, backend);
    else if (readCredential(key, backend) !== secret) throw Error('Existing credential differs; refusing adoption');
    // Recheck both stores before public activation. The journal is never rewritten.
    if (readCredential(key, backend) !== secret || JSON.stringify(readPrivate(journalPath(directory, agent))) !== JSON.stringify(expected)) throw Error('Partial provision changed during reconciliation');
    const { host, ownerSecret: _owner, ownerPublic: _public, agentSecret: _agent, ...harness } = setup;
    writePrivate(manifestPath(directory), { version: 3, host, ownerPublic, setups: { default: harness }, agents: { [agent]: { key, setup: 'default' } } }, true);
  });
}

/** Add an independent v3 identity to an existing immutable local binding. Failed
 * prefixes retain an inert journal for explicit reconciliation, never overwrite. */
export function addCredentialSlot(directory: string, secret: string, root: Genesis, setupId = 'default', expectedFingerprint: string | undefined, backend: CredentialBackend = systemCredentials): void {
  locked(directory, () => {
    const manifest = readCredentialManifest(directory), agent = publicKey(secret), genesis = validateGenesis(root);
    if (manifest.agents[agent] || Object.keys(manifest.agents).length >= 32) throw Error('Slot exists or installation full; retained keys require explicit import');
    if (manifest.retiredBindings?.[setupId]) throw Error('Binding retired');
    const harness = manifest.setups[setupId];
    if (!harness) throw Error('Unknown host harness setup');
    if (expectedFingerprint !== undefined && bindingFingerprint({ ...harness, host: manifest.host, ownerPublic: manifest.ownerPublic }) !== expectedFingerprint) throw Error('Binding definition changed; reopen local setup');
    if (genesis.agent !== agent || genesis.owner !== manifest.ownerPublic) throw Error('Genesis ownership mismatch');
    const setup = validateSetup({ ...harness, host: manifest.host, ownerPublic: manifest.ownerPublic, agentSecret: secret });
    if (!setupModels(setup).length) throw Error('Diagnostic-only binding cannot enroll an executable agent');
    if (existsSync(journalPath(directory, agent))) throw Error('Partial slot requires explicit reconciliation; refusing reset');
    writePrivate(journalPath(directory, agent), initialState(setup, genesis), true);
    const key = createCredential('agent', secret, backend);
    manifest.agents[agent] = { key, setup: setupId };
    writePrivate(manifestPath(directory), manifest);
  });
}

/** Public-only discovery of interrupted ADDED identities: an agent directory
 * holding a retained journal that no manifest entry activates. No credential is
 * read or created and no file is modified; journal binding/assignment contents
 * are already-public. Candidate bindings are derived from the journal's retained
 * selection, never defaulted: a definition change or corrupt state fails closed. */
export function orphanCredentialSlots(directory: string): { host: string; ownerPublic: string; orphans: { agent: string; genesis: Genesis; candidates: { setupId: string; fingerprint: string }[] }[] } {
  const manifest = readCredentialManifest(directory);
  const orphans: { agent: string; genesis: Genesis; candidates: { setupId: string; fingerprint: string }[] }[] = [];
  const agentsDirectory = join(directory, 'agents');
  if (!existsSync(agentsDirectory)) return { host: manifest.host, ownerPublic: manifest.ownerPublic, orphans };
  for (const agent of readdirSync(agentsDirectory)) {
    if (!/^[0-9a-f]{64}$/.test(agent) || Object.hasOwn(manifest.agents, agent) || !existsSync(journalPath(directory, agent))) continue;
    const state = object(readPrivate(journalPath(directory, agent)));
    const binding = object(state.binding), assignment = object(state.assignment), selected = object(state.selected);
    if (text(binding.agent) !== agent) throw Error(`Partial journal for ${agent} carries a foreign identity; refusing`);
    const genesis = validateGenesis(assignment.genesis);
    if (genesis.owner !== manifest.ownerPublic || genesis.agent !== agent) throw Error(`Partial journal for ${agent} is not owned by this installation; refusing`);
    const model = text(selected.model), workspace = text(selected.workspace);
    const candidates = Object.entries(manifest.setups)
      .filter(([, harness]) => { const setup = { ...harness, host: manifest.host, ownerPublic: manifest.ownerPublic }; return setupModels(setup)[0] === model && setup.workspace === workspace; })
      .map(([id, harness]) => ({ setupId: id, fingerprint: bindingFingerprint({ ...harness, host: manifest.host, ownerPublic: manifest.ownerPublic }) }));
    orphans.push({ agent, genesis, candidates });
  }
  return { host: manifest.host, ownerPublic: manifest.ownerPublic, orphans };
}

/** Explicit recovery of an interrupted ADDED identity on an active installation.
 * The chosen immutable binding and supplied genesis must reproduce the orphan
 * journal's exact untouched initial state; the journal is never reset, rewritten
 * or moved. An existing credential is adopted only when it equals the supplied
 * key; a missing one is created only by this explicit action, and both stores are
 * revalidated before the manifest entry activates. Active/public-only slots never
 * enter here; wrong or foreign inputs leave every retained byte inert. */
export function reconcileCredentialSlot(directory: string, setupId: string, secret: string, root: Genesis, expectedFingerprint: string | undefined, backend: CredentialBackend = systemCredentials): void {
  locked(directory, () => {
    const manifest = readCredentialManifest(directory), agent = publicKey(secret), genesis = validateGenesis(root);
    if (manifest.agents[agent]) throw Error('Retained slot exists; use explicit import-agent-key');
    if (Object.keys(manifest.agents).length >= 32) throw Error('Installation full; retained keys require explicit import');
    if (manifest.retiredBindings?.[setupId]) throw Error('Binding retired');
    const harness = manifest.setups[setupId];
    if (!harness) throw Error('Unknown host harness setup');
    if (expectedFingerprint !== undefined && bindingFingerprint({ ...harness, host: manifest.host, ownerPublic: manifest.ownerPublic }) !== expectedFingerprint) throw Error('Binding definition changed; reopen local setup');
    if (genesis.agent !== agent || genesis.owner !== manifest.ownerPublic) throw Error('Genesis ownership mismatch');
    const setup = validateSetup({ ...harness, host: manifest.host, ownerPublic: manifest.ownerPublic, agentSecret: secret });
    if (!setupModels(setup).length) throw Error('Diagnostic-only binding cannot enroll an executable agent');
    const path = journalPath(directory, agent);
    if (!existsSync(path)) throw Error('No interrupted added-slot journal for this identity; nothing to reconcile');
    const expected = initialState(setup, genesis);
    // Canonical equality: field order in a semantically identical genesis file is not identity.
    if (semanticHash(readPrivate(path)) !== semanticHash(expected)) throw Error('Partial journal differs from exact inert state; refusing reset');
    const key = credentialReference('agent', agent);
    const existing = backend.read(key);
    if (existing === null) createCredential('agent', secret, backend);
    else if (readCredential(key, backend) !== secret) throw Error('Existing credential differs; refusing adoption');
    // Recheck both stores before public activation; sibling journals are never touched.
    if (readCredential(key, backend) !== secret || semanticHash(readPrivate(path)) !== semanticHash(expected)) throw Error('Partial slot changed during reconciliation');
    manifest.agents[agent] = { key, setup: setupId };
    writePrivate(manifestPath(directory), manifest);
  });
}
