import { createCliRenderer } from '@opentui/core';
import { createReadStream, createWriteStream } from 'node:fs';
import { createInterface } from 'node:readline';
import { OpenTuiScreen, type ManagerAction } from './opentui-screen.ts';
import type { ManagerRequest, ManagerSnapshot } from './manager-controller.ts';

const renderer = await createCliRenderer({ exitOnCtrlC: false });
const screen = new OpenTuiScreen(renderer);
const output = createWriteStream('', { fd: 4 });
let snapshot: ManagerSnapshot = { local: [], agents: [], status: 'Connecting Node controller…' };
let scope = 0, selected = '', sequence = 0, lastStatus = '';
let pending: { id: number; resolve: () => void } | undefined;
function request(action: string, values?: Record<string,string>, target?: string, revision?: number) {
  if (pending) return Promise.resolve();
  const id = ++sequence;
  const completion = new Promise<void>(resolve => { pending = { id, resolve }; });
  const message: ManagerRequest = { id, action, values, target, revision };
  screen.setPending(true); screen.notice(`Working: ${action}… Esc cancels waiting, not submitted remote work or committed OS writes. Inspect before retry.`);
  output.write(JSON.stringify(message) + '\n');
  return completion;
}
async function routing(action: string, secret = false) {
  const retained = action !== 'configure' ? snapshot.routing : undefined;
  let owner = retained?.owner ?? '', relay = retained?.relay ?? '';
  if (!retained) {
    let step = 0;
    while (step < 2) {
      const label = step === 0 ? `${action === 'configure' ? 'Configure this computer' : 'Owner sign-in'} · 1 of 3\nOwner PUBLIC npub/hex (required)` : 'Configure routing · 2 of 3\nManagement relay URL (ws:// or wss://)';
      const answer = await screen.input(label, step === 0 ? owner : relay, false, false, value => step === 0 ? /^(?:[0-9a-fA-F]{64}|npub1[023456789acdefghjklmnpqrstuvwxyz]{58})$/.test(value.trim()) ? '' : 'Enter a public 64-hex key or npub.' : /^wss?:\/\//.test(value) ? '' : 'Enter a ws:// or wss:// relay URL.', step === 1 ? value => { relay = value; } : undefined);
      if (answer === undefined) return;
      if (answer === '\0back') { step = 0; continue; }
      if (step === 0) owner = answer; else relay = answer;
      step++;
    }
  }
  const values: Record<string,string> = { owner, relay };
  if (secret) {
    const key = await screen.input('Matching owner private key, 64 hex. Saved only in this computer’s OS credentials. Never sent to a host.', '', true);
    if (key === undefined) return;
    values.secret = key;
  }
  if (!await screen.confirm(action === 'configure' ? `Configure this computer · 3 of 3\nOwner: ${owner}\nRelay: ${relay}\nCreate host identity in OS credentials? No agent or Start.` : `Owner: ${owner}\nRelay: ${relay}\n${retained ? 'Retained routing (read-only). ' : ''}Access owner OS credential? Permission may prompt. Matching import only.`)) { delete values.secret; return; }
  await request(action, values); delete values.secret;
}
function eligibility(action: string) {
  const row = snapshot.agents.find(r => r.id === selected);
  if (!snapshot.owner) return 'Owner sign-in required.';
  if (!row?.disabled) return 'Select an agent observation.';
  return row.disabled[action] || (action === 'select-config' && !row.configurations.length ? 'No existing configurations reported.' : undefined);
}
function render() {
  screen.setOwner(snapshot.owner?.slice(-12));
  const rows = scope === 0 ? snapshot.local : snapshot.agents;
  if (selected === 'empty') selected = '';
  if (!selected || (scope === 0 && selected === 'missing' && rows.some(r => r.id === 'host'))) selected = rows[0]?.id ?? '';
  const vanished = !!selected && !rows.some(row => row.id === selected);
  const actions: ManagerAction[] = scope === 0 ? [
    snapshot.local.some(r => r.id === 'host') ? { label: 'Host configuration saved — inspect', run: () => screen.inspect() } : { label: 'Configure this computer', disabled: snapshot.local.some(r => r.id === 'error') ? 'Retained configuration needs attention — preserved, not reset.' : undefined, run: () => routing('configure') },
    { label: 'Provision from prepared binding/genesis files', disabled: snapshot.local.some(r => r.id === 'host') ? undefined : 'Configure this computer first.', run: async () => {
      const binding = await screen.input('Absolute existing local binding JSON path (no identity fields)'); if (binding === undefined) return;
      const genesis = await screen.input('Absolute owner-authorized agent genesis JSON path'); if (genesis === undefined) return;
      const secret = await screen.input('Matching AGENT private key, 64 hex. Stored in host OS credentials, not the owner key.', '', true); if (secret === undefined) return;
      if (!await screen.confirm('Provision STOPPED agent into this fresh local installation? Existing or partial installations refuse reset. OS access may prompt.')) return;
      await request('provision', { binding, genesis, secret });
    } },
    { label: 'Refresh local inspection', run: () => request('refresh') },
    { label: 'Save private instructions draft', run: async () => {
      const name = await screen.input('New private instruction draft · 1 of 2\nProfile name', '', false, false, v => v.trim() ? '' : 'A name is required.'); if (name === undefined) return;
      const instructions = await screen.input(`Draft: ${name}\nLocal only — not published or applied. Private instructions (not credentials)`, '', false, true, v => v.trim() ? '' : 'Instructions are required.'); if (instructions === undefined) return;
      await request('draft', { name, instructions });
    } },
    { label: 'Provisioning / service commands', run: () => screen.notice('Forms: Not available in this build. Existing CLI: beehive local-setup ~/.beehive/host; beehive provision-agent; beehive host --owner-present. Quit this manager before foreground CLI use.') },
  ] : snapshot.owner ? [
    { label: 'Choose next configuration…', disabled: eligibility('select-config'), run: async () => {
      const row = snapshot.agents.find(r => r.id === selected); if (!row) return;
      const name = await screen.choose('Choose existing next configuration', row.configurations); if (name === undefined) return;
      if (!await screen.confirm(`Select ${name} for exact host/agent ${row.id} at revision ${row.revision}? Actual run unchanged.`)) return;
      await request('select-config', { name }, row.id, row.revision);
    } },
    ...(['start','stop','restart'] as const).map(action => ({ label: action === 'restart' ? 'Restart — not available (binding safety)' : `${action === 'start' ? 'Start' : 'Stop'} selected agent`, disabled: action === 'restart' ? 'Backend Restart may switch bindings. Use Stop, inspect accepted stopped report, then Start. No Restart request is sent.' : eligibility(action), run: async () => {
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
  if (scope === 1 && !snapshot.owner) actions.push({ label: 'New private instruction draft…', run: async () => { const name = await screen.input('New private instruction draft · 1 of 2\nName', '', false, false, v => v.trim() ? '' : 'Name required.'); if (name === undefined) return; const instructions = await screen.input(`Draft: ${name}\nLocal only — not published or applied`, '', false, true, v => v.trim() ? '' : 'Instructions required.'); if (instructions !== undefined) await request('draft', { name, instructions }); } });
  actions.push({ label: 'Quit manager (not Stop)', run: () => screen.close() });
  screen.show(vanished ? [{ id: selected, label: 'Selection no longer available', detail: 'Selection no longer available. Choose a current observation; no operation is targeted at a replacement row.' }, ...rows] : scope === 0 ? snapshot.local : snapshot.agents.length ? snapshot.agents : [{ id: 'empty', label: snapshot.owner ? 'No host observations yet' : 'Owner sign-in required', detail: 'Agents management is owner-authorized. Inventory comes from privately advertising hosts, NOT an independent relay agent catalog. Empty/offline observations do not prove agents stopped. Local Host and offline drafts do not require sign-in.' }], actions, selected);
  if (lastStatus !== snapshot.status) { lastStatus = snapshot.status; screen.notice(snapshot.status); }
}
screen.onScope = value => { scope = value; selected = ''; render(); };
screen.onSelect = id => { if (selected !== id) { selected = id; render(); } };
const input = createInterface({ input: createReadStream('', { fd: 3 }) });
input.on('line', line => {
  try {
    const value = JSON.parse(line);
    if (value.snapshot) { snapshot = value.snapshot; render(); }
    if (value.complete === pending?.id) { const old = pending; pending = undefined; screen.setPending(false); old?.resolve(); }
  } catch { screen.notice('Controller response unavailable. Quit and inspect retained state.'); }
});
input.on('close', () => screen.close());
screen.onCancel = () => { if (pending) output.write(JSON.stringify({ cancel: pending.id }) + '\n'); };
await screen.done;
output.end(JSON.stringify({ quit: true }) + '\n'); input.close();
process.exit(0);
