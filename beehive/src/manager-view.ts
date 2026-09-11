import { createCliRenderer } from '@opentui/core';
import { createReadStream, createWriteStream } from 'node:fs';
import { createInterface } from 'node:readline';
import { OpenTuiScreen, type ManagerAction } from './opentui-screen.ts';
import type { ManagerRequest, ManagerSnapshot } from './manager-controller.ts';

const renderer = await createCliRenderer({ exitOnCtrlC: false });
const screen = new OpenTuiScreen(renderer);
const output = createWriteStream('', { fd: 4 });
let snapshot: ManagerSnapshot = { local: [], agents: [], status: 'Connecting Node controller…' };
let scope = 0, selected = '', sequence = 0;
let pending: { id: number; resolve: () => void } | undefined;
function request(action: string, values?: Record<string,string>, target?: string, revision?: number) {
  if (pending) return Promise.resolve();
  const id = ++sequence;
  const completion = new Promise<void>(resolve => { pending = { id, resolve }; });
  const message: ManagerRequest = { id, action, values, target, revision };
  output.write(JSON.stringify(message) + '\n');
  return completion;
}
async function routing(action: string, secret = false) {
  const owner = await screen.input('Owner PUBLIC npub/hex (blank uses retained controller owner)');
  if (owner === undefined) return;
  const relay = await screen.input('Management relay URL (blank uses retained controller relay)');
  if (relay === undefined) return;
  const values: Record<string,string> = { owner, relay };
  if (secret) {
    const key = await screen.input('Matching owner private key, 64 hex. Saved only in this computer’s OS credentials. Never sent to a host.', '', true);
    if (key === undefined) return;
    values.secret = key;
  }
  if (!await screen.confirm(action === 'configure' ? 'Create this host identity in OS credentials? No owner key, agent or Start.' : 'Access this computer’s owner OS credential? Permission may prompt. Import is verified against configured owner.')) { delete values.secret; return; }
  await request(action, values); delete values.secret;
}
function render() {
  screen.setOwner(snapshot.owner?.slice(-12));
  const rows = scope === 0 ? snapshot.local : snapshot.agents;
  if (!rows.some(row => row.id === selected)) selected = rows[0]?.id ?? '';
  const actions: ManagerAction[] = scope === 0 ? [
    { label: 'Configure missing Local Host', run: () => routing('configure') },
    { label: 'Provision agent from existing binding/genesis', run: async () => {
      const binding = await screen.input('Absolute existing local binding JSON path (no identity fields)'); if (binding === undefined) return;
      const genesis = await screen.input('Absolute owner-authorized agent genesis JSON path'); if (genesis === undefined) return;
      const secret = await screen.input('Matching AGENT private key, 64 hex. Stored in host OS credentials, not the owner key.', '', true); if (secret === undefined) return;
      if (!await screen.confirm('Provision STOPPED agent into this fresh local installation? Existing or partial installations refuse reset. OS access may prompt.')) return;
      await request('provision', { binding, genesis, secret });
    } },
    { label: 'Refresh local inspection', run: () => request('refresh') },
    { label: 'Save private instructions draft', run: async () => {
      const name = await screen.input('Profile name'); if (name === undefined) return;
      const instructions = await screen.input('Private behavior instructions (not credentials)', '', false, true); if (instructions === undefined) return;
      await request('draft', { name, instructions });
    } },
    { label: 'Provisioning / service commands', run: () => screen.notice('Forms: Not available in this build. Existing CLI: beehive local-setup ~/.beehive/host; beehive provision-agent; beehive host --owner-present. Quit this manager before foreground CLI use.') },
  ] : snapshot.owner ? [
    { label: 'Select existing next configuration', run: async () => {
      const row = snapshot.agents.find(r => r.id === selected); if (!row) return;
      const name = await screen.input(`Existing configurations: ${row.configurations.join(', ')}\nExact name`); if (name === undefined) return;
      if (!await screen.confirm(`Select ${name} for ${row.label}? Actual run unchanged.`)) return;
      await request('select-config', { name }, row.id, row.revision);
    } },
    ...(['start','stop','restart'] as const).map(action => ({ label: action === 'restart' ? 'Restart — not available (binding safety)' : `${action === 'start' ? 'Start' : 'Stop'} selected agent`, run: async () => {
      const row = snapshot.agents.find(r => r.id === selected); if (!row) return;
      if (!await screen.confirm(`${action.toUpperCase()} exact agent/host ${row.id} at revision ${row.revision}?`)) return;
      await request(action, undefined, row.id, row.revision);
    } })),
    { label: 'Inspect operation receipts', run: () => request('operations') },
    { label: 'Reconcile (not retry)', run: () => request('reconcile') },
    { label: 'Sign out — not Stop', run: () => request('signout') },
    { label: 'Public metadata / profile publication', run: () => screen.notice('Publication forms: Not available in this build. Existing CLI: beehive drafts ~/.beehive/owner; beehive tui discover <relay> ~/.beehive/owner. Generated keys and independent agent catalog are not implemented.') },
  ] : [
    { label: 'Sign in with saved owner key', run: () => routing('signin') },
    { label: 'Import matching owner key and sign in', run: () => routing('signin', true) },
  ];
  screen.show(scope === 0 ? snapshot.local : snapshot.agents.length ? snapshot.agents : [{ id: 'empty', label: snapshot.owner ? 'No host observations yet' : 'Owner sign-in required', detail: 'Agents management is owner-authorized. Inventory comes from privately advertising hosts, NOT an independent relay agent catalog. Empty/offline observations do not prove agents stopped. Local Host and offline drafts do not require sign-in.' }], actions, selected);
  screen.notice(snapshot.status);
}
screen.onScope = value => { scope = value; selected = ''; render(); };
screen.onSelect = id => { selected = id; };
const input = createInterface({ input: createReadStream('', { fd: 3 }) });
input.on('line', line => {
  try {
    const value = JSON.parse(line);
    if (value.snapshot) { snapshot = value.snapshot; render(); }
    if (value.complete === pending?.id) { const old = pending; pending = undefined; old?.resolve(); }
  } catch { screen.notice('Controller response unavailable. Quit and inspect retained state.'); }
});
input.on('close', () => screen.close());
renderer.keyInput.on('keypress', key => {
  if (key.name === 'escape' && pending) output.write(JSON.stringify({ cancel: pending.id }) + '\n');
});
await screen.done;
output.end(JSON.stringify({ quit: true }) + '\n'); input.close();
process.exit(0);
