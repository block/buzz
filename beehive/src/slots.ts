import { existsSync, mkdirSync, rmdirSync } from 'node:fs';
import { join } from 'node:path';
import { validateSetup, initialState, loadSlotState, setupModels, bindingFingerprint, type Setup } from './host.ts';
import { readPrivate, writePrivate } from './storage.ts';
import { object, publicKey, text } from './protocol.ts';
import { validateGenesis, type Genesis } from './assignment.ts';
import { prepareConversation, type ConversationSetup } from './conversation.ts';
import { semanticHash } from './handoff.ts';

type Harness = Omit<Setup, 'host' | 'ownerSecret' | 'agentSecret'>;
/** A removed local key copy leaves a public-only slot: `secret: null` retains identity. */
type AgentEntry = { secret: string | null; setup: string; legacy?: true };
type Installation = { conversation?: ConversationSetup; version: 2; host: string; ownerSecret: string; setups: Record<string, Harness>; agents: Record<string, AgentEntry> };
export type SlotEntry = { setup: Setup; path: string; setupId: string; agent: string; keyPresent: boolean; bindings: Record<string, Setup> };
function slotPath(directory: string, key: string, legacy: boolean | undefined) {
  return legacy ? join(directory, 'journal.json') : join(directory, 'agents', key, 'journal.json');
}
function readInstallation(directory: string): Installation {
  const i = object(readPrivate(join(directory, 'setup.json'))) as Installation;
  if (i.version !== 2) throw Error('Explicit stopped migrate-slots required');
  const keys = Object.keys(object(i.agents));
  if (!keys.length || keys.length > 32) throw Error('Installation requires 1..32 slots');
  object(i.setups);
  // Shared harness inventory must never carry key material: a setups entry that did
  // would silently re-arm a removed-key public-only slot at load time.
  for (const name of Object.keys(i.setups)) {
    const harness = object(i.setups[name]);
    if (Object.hasOwn(harness, 'host') || Object.hasOwn(harness, 'ownerSecret') || Object.hasOwn(harness, 'agentSecret')) throw Error('Shared harness inventory must not copy key material');
  }
  // One pinned installation authority. Legacy installations derive it from default.
  // Embedded binding snapshots are immutable mode opt-ins, never alternate authority.
  const common = i.conversation ?? i.setups.default?.conversation;
  for (const harness of Object.values(i.setups)) {
    if (harness.conversation && semanticHash(harness.conversation) !== semanticHash(common ?? null)) throw Error('Binding cannot change conversation authority');
  }
  for (const key of keys) {
    if (!/^[0-9a-f]{64}$/.test(key)) throw Error('Invalid slot identity');
    const a = object(i.agents[key]) as AgentEntry;
    if (!Object.hasOwn(i.setups, a.setup)) throw Error('Invalid slot binding');
    if (a.secret !== null) {
      if (publicKey(a.secret) !== key) throw Error('Invalid slot binding');
      validateSetup({ ...i.setups[a.setup], host: i.host, ownerSecret: i.ownerSecret, agentSecret: a.secret });
    } else {
      // Public-only slot: identity is the retained journal binding, never a new key.
      const binding = object(object(readPrivate(slotPath(directory, key, a.legacy))).binding);
      if (binding.host !== i.host || binding.owner !== publicKey(i.ownerSecret) || binding.agent !== key) throw Error('Public slot binding mismatch');
      validateSetup({ ...i.setups[a.setup], host: i.host, ownerSecret: i.ownerSecret });
    }
  }
  return i;
}
/** Resolve shared local harness inventory without copying agent keys into setups. */
export function installationSlots(directory: string): SlotEntry[] {
  const raw = object(readPrivate(join(directory, 'setup.json')));
  if (raw.version !== 2) {
    const setup = validateSetup(raw);
    if (setup.agentSecret === undefined) throw Error('Legacy setup requires its agent key; explicit migrate-slots first');
    return [{ setupId: 'default', setup, path: join(directory, 'journal.json'), agent: publicKey(setup.agentSecret), keyPresent: true, bindings: { default: setup } }];
  }
  const i = readInstallation(directory);
  return Object.entries(i.agents).map(([key, a]) => ({
    setupId: a.setup,
    bindings: Object.fromEntries(Object.entries(i.setups).map(([id, harness]) => [id, validateSetup({ ...harness, host: i.host, ownerSecret: i.ownerSecret, ...(a.secret === null ? {} : { agentSecret: a.secret }) })])),
    setup: validateSetup({ ...i.setups[a.setup], host: i.host, ownerSecret: i.ownerSecret, ...(a.secret === null ? {} : { agentSecret: a.secret }) }),
    path: a.legacy ? join(directory, 'journal.json') : join(directory, 'agents', key, 'journal.json'),
    agent: key,
    keyPresent: a.secret !== null,
  }));
}
/** Explicit single stopped upgrade: one atomic setup replacement; journal stays byte-identical. */
export function migrateSlots(directory: string): void {
  const lock = join(directory, 'host.lock'); mkdirSync(lock, { mode: 0o700 });
  try {
    const raw = object(readPrivate(join(directory, 'setup.json')));
    if (raw.version === 2) throw Error('Slots already enabled');
    const s = validateSetup(raw);
    if (s.agentSecret === undefined) throw Error('Legacy setup requires its agent key; public-only slots need migrate-slots first');
    const state = object(readPrivate(join(directory, 'journal.json')));
    const binding = object(state.binding); const assignment = object(state.assignment);
    const genesis = validateGenesis(assignment.genesis);
    if (state.phase !== 'stopped' || state.actual !== null || binding.host !== s.host || binding.owner !== publicKey(s.ownerSecret) || binding.agent !== publicKey(s.agentSecret) || genesis.owner !== binding.owner || genesis.agent !== binding.agent || assignment.assignedHost !== genesis.initialHost) throw Error('Slot migration requires matching stopped assignment');
    const { host, ownerSecret, agentSecret, ...harness } = s;
    const i: Installation = { ...(harness.conversation ? { conversation: structuredClone(harness.conversation) } : {}), version: 2, host, ownerSecret, setups: { default: harness }, agents: { [publicKey(agentSecret)]: { secret: agentSecret, setup: 'default', legacy: true } } };
    writePrivate(join(directory, 'setup.json'), i);
  } finally { rmdirSync(lock); }
}
/** Local key enrollment reuses a host-owned harness, never another agent's key. */
export function addSlot(directory: string, secret: string, genesis: Genesis, setupName = 'default', expectedFingerprint?: string): void {
  const lock = join(directory, 'host.lock'); mkdirSync(lock, { mode: 0o700 });
  try {
    const i = readInstallation(directory); const key = publicKey(secret);
    const existing = i.agents[key];
    if (existing?.secret === null) throw Error('Public-only slot retains this identity; local key repair is explicit local reconciliation, never slot re-creation');
    if (Object.keys(i.agents).length >= 32 || existing) throw Error('Slot exists or installation full');
    if (!Object.hasOwn(i.setups, setupName)) throw Error('Unknown host harness setup');
    if (expectedFingerprint !== undefined && semanticHash(i.setups[setupName]) !== expectedFingerprint) throw Error('Binding definition changed; reopen local setup');
    const setup = validateSetup({ ...i.setups[setupName], host: i.host, ownerSecret: i.ownerSecret, agentSecret: secret });
    const slotDirectory = join(directory, 'agents', key);
    if (existsSync(slotDirectory)) throw Error('Partial slot requires local reconciliation; cannot reset');
    // The journal is inert until the manifest activates it; never copy setup/keys.
    provisionSlotJournal(slotDirectory, setup, genesis);
    i.agents[key] = { secret, setup: setupName };
    writePrivate(join(directory, 'setup.json'), i);
  } finally { rmdirSync(lock); }
}
function provisionSlotJournal(directory: string, setup: Setup, root: Genesis) {
  const genesis = validateGenesis(root);
  if (setup.agentSecret === undefined) throw Error('Slot enrollment requires its agent key');
  if (genesis.owner !== publicKey(setup.ownerSecret) || genesis.agent !== publicKey(setup.agentSecret)) throw Error('Genesis ownership mismatch');
  writePrivate(join(directory, 'journal.json'), initialState(setup, genesis), true);
}
/**
 * Deliberate LOCAL removal of one agent's key copy. The public slot stays: identity,
 * genesis/assignment, configurations and the complete journal/receipt/run history are
 * retained byte-identically; siblings and the owner identity are untouched. Only the
 * local secret copy is deleted — this is not a global cryptographic revocation, and no
 * remote operation recreates it. Requires the installation lock (refuses a running host
 * or an unclean exit without any PID or stale-lock removal) and a stopped slot.
 */
export function removeSlotKey(directory: string, agentKey: string): void {
  if (!/^[0-9a-f]{64}$/.test(agentKey)) throw Error('Invalid agent public key');
  const lock = join(directory, 'host.lock');
  if (existsSync(lock)) throw Error('Installation lock present (running host or unclean exit); local key removal refuses without PID or stale-lock removal');
  mkdirSync(lock, { mode: 0o700 });
  try {
    const raw = object(readPrivate(join(directory, 'setup.json')));
    if (raw.version !== 2) throw Error('Explicit stopped migrate-slots required before key removal');
    const i = readInstallation(directory);
    const entry = i.agents[agentKey];
    if (!entry) throw Error('Unknown agent slot on this installation');
    if (entry.secret === null) throw Error('Agent key already removed locally; public slot retained');
    const setup = validateSetup({ ...i.setups[entry.setup], host: i.host, ownerSecret: i.ownerSecret });
    const state = loadSlotState(setup, slotPath(directory, agentKey, entry.legacy), agentKey);
    if (state.phase !== 'stopped' || state.actual !== null) throw Error('Key removal requires a stopped slot with no actual run');
    entry.secret = null;
    writePrivate(join(directory, 'setup.json'), i);
  } finally { rmdirSync(lock); }
}

/** Explicit local restoration of a retained public identity, never enrollment or
 * assignment repair. Importing on standby/consumed source grants no execution. */
export function importSlotKey(directory: string, agentKey: string, secret: string): void {
  if (!/^[0-9a-f]{64}$/.test(agentKey) || publicKey(secret) !== agentKey) throw Error('Imported key does not match the selected public identity');
  const lock = join(directory, 'host.lock');
  mkdirSync(lock, { mode: 0o700 });
  try {
    const i = readInstallation(directory);
    const entry = i.agents[agentKey];
    if (!entry) throw Error('Unknown retained public slot; import cannot invent authority');
    if (entry.secret !== null) throw Error('Local key already present; reuse the existing identity without import');
    const setup = validateSetup({ ...i.setups[entry.setup], host: i.host, ownerSecret: i.ownerSecret });
    const state = loadSlotState(setup, slotPath(directory, agentKey, entry.legacy), agentKey);
    if (state.phase !== 'stopped' || state.actual !== null) throw Error('Key import requires a stopped slot with no actual run');
    entry.secret = secret;
    writePrivate(join(directory, 'setup.json'), i);
  } finally { rmdirSync(lock); }
}

/** Provision an immutable reusable local binding under the service-start lock.
 * No identity, journal, selected configuration or running process is changed. */
export function addHarnessBinding(directory: string, id: string, value: Harness, source?: { id: string; fingerprint: string }): void {
  if (!/^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/.test(id) || ['constructor', 'prototype', '__proto__'].includes(id)) throw Error('Invalid binding ID');
  const lock = join(directory, 'host.lock'); mkdirSync(lock, { mode: 0o700 });
  try {
    const i = readInstallation(directory);
    if (Object.hasOwn(i.setups, id) || Object.keys(i.setups).length >= 32) throw Error('Binding exists or inventory full');
    if (source && (!Object.hasOwn(i.setups, source.id) || semanticHash(i.setups[source.id]) !== source.fingerprint)) throw Error('Binding definition changed; reopen local setup');
    const raw = object(value);
    if (['host', 'ownerSecret', 'agentSecret'].some(k => Object.hasOwn(raw, k))) throw Error('Binding cannot carry identity');
    validateSetup({ ...raw, host: i.host, ownerSecret: i.ownerSecret });
    // Conversation authority is installation-owned, not a binding selector.
    if (value.conversation && semanticHash(value.conversation) !== semanticHash(i.conversation ?? i.setups.default?.conversation ?? null)) throw Error('Binding cannot change conversation authority');
    i.setups[id] = structuredClone(value);
    writePrivate(join(directory, 'setup.json'), i);
  } finally { rmdirSync(lock); }
}

/** Atomic local activation of a NEW normal binding. No journal/selection changes:
 * remote Save retains its ordinary public revision CAS, then explicit Start/Restart.
 * Old diagnostic references retain their exact meaning. Common authority is pinned
 * once; this action cannot retarget another normal binding's runtime/tool/trust.
 */
export function addConversationBinding(directory: string, agent: string, source: { id: string; fingerprint: string }, id: string, conversation: ConversationSetup): void {
  if (!/^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/.test(id) || ['constructor', 'prototype', '__proto__'].includes(id)) throw Error('Invalid binding ID');
  const lock = join(directory, 'host.lock'); mkdirSync(lock, { mode: 0o700 });
  try {
    const i = readInstallation(directory), entries = installationSlots(directory);
    for (const entry of entries) {
      const state = loadSlotState(entry.setup, entry.path, entry.agent);
      if (state.phase !== 'stopped' || state.actual !== null) throw Error('Conversation conversion requires every slot stopped');
    }
    const entry = entries.find(e => e.agent === agent), setup = entry?.bindings[source.id];
    if (!entry?.keyPresent || !setup?.agentSecret) throw Error('Retained agent key required; no key recreation');
    if (bindingFingerprint(setup) !== source.fingerprint) throw Error('Binding definition changed; reopen local setup');
    if (setup.mode === 'fixture') throw Error('Choose an ACP binding, not fixture');
    if (Object.hasOwn(i.setups, id) || Object.keys(i.setups).length >= 32) throw Error('Binding exists or inventory full');
    const common = i.conversation ?? i.setups.default?.conversation;
    if (common && semanticHash(common) !== semanticHash(conversation)) throw Error('Installation conversation authority already pinned; cannot retarget it');
    prepareConversation(conversation, { executable: setup.runner, args: setup.args, workspace: setup.workspace,
      home: text(setup.serviceHome), configDirectory: text(setup.configDirectory),
      databricksHost: setup.mode === 'goose' || setup.mode === 'claude' ? '' : text(setup.databricksHost),
      ...(setup.mode === 'claude' ? { harness: 'claude' as const, claude: setup.claude } : {}), ...(setup.mode === 'goose' ? { harness: 'goose' as const, provider: setup.gooseProvider } : {}), model: setupModels(setup)[0]!
    }, setup.agentSecret, publicKey(setup.ownerSecret));
    i.conversation = structuredClone(conversation);
    i.setups[id] = { ...i.setups[source.id]!, conversation: structuredClone(conversation) };
    // Single fsynced manifest rename activates authority and new definition together.
    // A crash cannot expose a half-converted selected journal or rewrite history.
    writePrivate(join(directory, 'setup.json'), i);
  } finally { rmdirSync(lock); }
}
