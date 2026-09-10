import { existsSync, mkdirSync, rmdirSync } from 'node:fs';
import { join } from 'node:path';
import { validateSetup, initialState, type Setup } from './host.ts';
import { readPrivate, writePrivate } from './storage.ts';
import { object, publicKey } from './protocol.ts';
import { validateGenesis, type Genesis } from './assignment.ts';

type Harness = Omit<Setup, 'host' | 'ownerSecret' | 'agentSecret'>;
type Installation = { version: 2; host: string; ownerSecret: string; setups: Record<string, Harness>; agents: Record<string, { secret: string; setup: string; legacy?: true }> };
function readInstallation(directory: string): Installation {
  const i = object(readPrivate(join(directory, 'setup.json'))) as Installation;
  if (i.version !== 2) throw Error('Explicit stopped migrate-slots required');
  const keys = Object.keys(object(i.agents));
  if (!keys.length || keys.length > 32) throw Error('Installation requires 1..32 slots');
  object(i.setups);
  for (const key of keys) {
    const a = i.agents[key]!;
    if (publicKey(a.secret) !== key || !Object.hasOwn(i.setups, a.setup)) throw Error('Invalid slot binding');
    validateSetup({ ...i.setups[a.setup], host: i.host, ownerSecret: i.ownerSecret, agentSecret: a.secret });
  }
  return i;
}
/** Resolve shared local harness inventory without copying agent keys into setups. */
export function installationSlots(directory: string): { setup: Setup; path: string }[] {
  const raw = object(readPrivate(join(directory, 'setup.json')));
  if (raw.version !== 2) return [{ setup: validateSetup(raw), path: join(directory, 'journal.json') }];
  const i = readInstallation(directory);
  return Object.entries(i.agents).map(([key, a]) => ({
    setup: validateSetup({ ...i.setups[a.setup], host: i.host, ownerSecret: i.ownerSecret, agentSecret: a.secret }),
    path: a.legacy ? join(directory, 'journal.json') : join(directory, 'agents', key, 'journal.json'),
  }));
}
/** Explicit single stopped upgrade: one atomic setup replacement; journal stays byte-identical. */
export function migrateSlots(directory: string): void {
  const lock = join(directory, 'host.lock'); mkdirSync(lock, { mode: 0o700 });
  try {
    const raw = object(readPrivate(join(directory, 'setup.json')));
    if (raw.version === 2) throw Error('Slots already enabled');
    const s = validateSetup(raw);
    const state = object(readPrivate(join(directory, 'journal.json')));
    const binding = object(state.binding); const assignment = object(state.assignment);
    const genesis = validateGenesis(assignment.genesis);
    if (state.phase !== 'stopped' || state.actual !== null || binding.host !== s.host || binding.owner !== publicKey(s.ownerSecret) || binding.agent !== publicKey(s.agentSecret) || genesis.owner !== binding.owner || genesis.agent !== binding.agent || assignment.assignedHost !== genesis.initialHost) throw Error('Slot migration requires matching stopped assignment');
    const { host, ownerSecret, agentSecret, ...harness } = s;
    const i: Installation = { version: 2, host, ownerSecret, setups: { default: harness }, agents: { [publicKey(agentSecret)]: { secret: agentSecret, setup: 'default', legacy: true } } };
    writePrivate(join(directory, 'setup.json'), i);
  } finally { rmdirSync(lock); }
}
/** Local key enrollment reuses a host-owned harness, never another agent's key. */
export function addSlot(directory: string, secret: string, genesis: Genesis, setupName = 'default'): void {
  const lock = join(directory, 'host.lock'); mkdirSync(lock, { mode: 0o700 });
  try {
    const i = readInstallation(directory); const key = publicKey(secret);
    if (Object.keys(i.agents).length >= 32 || Object.hasOwn(i.agents, key)) throw Error('Slot exists or installation full');
    if (!Object.hasOwn(i.setups, setupName)) throw Error('Unknown host harness setup');
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
  if (genesis.owner !== publicKey(setup.ownerSecret) || genesis.agent !== publicKey(setup.agentSecret)) throw Error('Genesis ownership mismatch');
  writePrivate(join(directory, 'journal.json'), initialState(setup, genesis), true);
}

/** Caller holds the installation lock; update shared harness inventory without key copies. */
export function saveDefaultHarness(directory: string, setup: Setup): void {
  const raw = object(readPrivate(join(directory, 'setup.json')));
  if (raw.version !== 2) { writePrivate(join(directory, 'setup.json'), setup); return; }
  const i = readInstallation(directory);
  const { host: _host, ownerSecret: _owner, agentSecret: _agent, ...harness } = setup;
  i.setups.default = harness;
  writePrivate(join(directory, 'setup.json'), i);
}
