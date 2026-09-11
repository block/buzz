import { createCliRenderer } from '@opentui/core';
import { createReadStream, createWriteStream } from 'node:fs';
import { createInterface } from 'node:readline';
import { OpenTuiScreen, type ManagerAction } from './opentui-screen.ts';
import type { ManagerRequest, ManagerSnapshot } from './manager-controller.ts';

const renderer = await createCliRenderer({ exitOnCtrlC: false });
const screen = new OpenTuiScreen(renderer);
const output = createWriteStream('', { fd: 4 });
let snapshot: ManagerSnapshot = { local: [], agents: [], status: 'Connecting to Beehive…' };
let scope = 0, selected = '', sequence = 0, lastStatus = '';
let pending: { id: number; resolve: () => void } | undefined;
function request(action: string, values?: Record<string,string>, target?: string, revision?: number) {
  if (pending) return Promise.resolve();
  const id = ++sequence;
  const completion = new Promise<void>(resolve => { pending = { id, resolve }; });
  const message: ManagerRequest = { id, action, values, target, revision };
  screen.setPending(true); screen.notice('Working… Esc stops waiting. Remote work and saved changes are not cancelled. Inspect before you try again.');
  output.write(JSON.stringify(message) + '\n');
  return completion;
}
async function routing(action: string, secret = false) {
  const retained = action !== 'configure' ? snapshot.routing : undefined;
  let owner = retained?.owner ?? '', relay = retained?.relay ?? '';
  if (!retained) {
    let step = 0;
    while (step < 2) {
      const label = step === 0 ? `${action === 'configure' ? 'Configure this computer' : 'Sign in as owner'} · 1 of 3\nOwner public key (npub or 64 hex characters). Do not enter a private key.` : 'Relay · 2 of 3\nRelay URL (ws:// or wss://)';
      const answer = await screen.input(label, step === 0 ? owner : relay, false, false, value => step === 0 ? /^(?:[0-9a-fA-F]{64}|npub1[023456789acdefghjklmnpqrstuvwxyz]{58})$/.test(value.trim()) ? '' : 'Enter an npub or a public key with 64 hex characters.' : /^wss?:\/\//.test(value) ? '' : 'Enter a ws:// or wss:// relay URL.', step === 1 ? value => { relay = value; } : undefined);
      if (answer === undefined) return;
      if (answer === '\0back') { step = 0; continue; }
      if (step === 0) owner = answer; else relay = answer;
      step++;
    }
  }
  const values: Record<string,string> = { owner, relay };
  if (secret) {
    const key = await screen.input('Owner private key (64 hex characters). It must match the owner public key. Saved in this computer’s secure credential store. Never sent to a host.', '', true);
    if (key === undefined) return;
    values.secret = key;
  }
  if (!await screen.confirm(action === 'configure' ? `Configure this computer · 3 of 3\nOwner: ${owner}\nRelay: ${relay}\nCreate a host identity in this computer’s secure credential store? This does not create an agent or start a host.` : `Owner: ${owner}\nRelay: ${relay}\n${retained ? 'Saved owner and relay cannot change here.\n' : ''}Access the owner key in this computer’s secure credential store? The system may ask for permission. An imported key must match the owner.`)) { delete values.secret; return; }
  await request(action, values); delete values.secret;
}
function eligibility(action: string) {
  const row = snapshot.agents.find(r => r.id === selected);
  if (!snapshot.owner) return 'Owner sign-in required.';
  if (!row?.disabled) return 'Select an agent.';
  return row.disabled[action] || (action === 'select-config' && !row.configurations.length ? 'The host reported no saved configurations.' : undefined);
}
function render() {
  screen.setOwner(snapshot.owner?.slice(-12));
  const rows = scope === 0 ? snapshot.local : snapshot.agents;
  if (selected === 'empty') selected = '';
  if (!selected || (scope === 0 && selected === 'missing' && rows.some(r => r.id === 'host'))) selected = rows[0]?.id ?? '';
  const vanished = !!selected && !rows.some(row => row.id === selected);
  const actions: ManagerAction[] = scope === 0 ? [
    snapshot.local.some(r => r.id === 'host') ? { label: 'Inspect host configuration', run: () => screen.inspect() } : { label: 'Configure this computer', disabled: snapshot.local.some(r => r.id === 'error') ? 'Saved configuration needs attention. It has not been reset.' : undefined, run: () => routing('configure') },
    { label: 'Add agent from prepared files', disabled: snapshot.local.some(r => r.id === 'host') ? undefined : 'Configure this computer first.', run: async () => {
      const binding = await screen.input('Local setup file (binding JSON). Enter its full path. The file must not contain host or agent identity fields.'); if (binding === undefined) return;
      const genesis = await screen.input('Agent authorization file (genesis JSON). Enter its full path. The file must be authorized by the owner.'); if (genesis === undefined) return;
      const secret = await screen.input('Agent private key (64 hex characters). It must match the agent. Saved in this host’s secure credential store. Do not enter the owner key.', '', true); if (secret === undefined) return;
      if (!await screen.confirm('Add a stopped agent to this new local installation? Existing or incomplete installations cannot be reset here. The system may ask for permission.')) return;
      await request('provision', { binding, genesis, secret });
    } },
    { label: 'Refresh local details', run: () => request('refresh') },
    { label: 'New private instructions draft…', run: async () => {
      const name = await screen.input('Private instructions · 1 of 2\nDraft name', '', false, false, v => v.trim() ? '' : 'Enter a draft name.'); if (name === undefined) return;
      const instructions = await screen.input(`Private instructions · 2 of 2\nDraft: ${name}\nSaved only on this computer. Not published or used by an agent. Do not enter passwords or keys.`, '', false, true, v => v.trim() ? '' : 'Enter instructions.'); if (instructions === undefined) return;
      await request('draft', { name, instructions });
    } },
    { label: 'Host setup commands', run: () => screen.notice('These forms are unavailable. Use the CLI:\nbeehive local-setup ~/.beehive/host\nbeehive provision-agent\nbeehive host --owner-present\nQuit Beehive before you use a foreground CLI command.') },
  ] : snapshot.owner ? [
    { label: 'Choose configuration…', disabled: eligibility('select-config'), run: async () => {
      const row = snapshot.agents.find(r => r.id === selected); if (!row) return;
      const name = await screen.choose('Configuration for next start', row.configurations); if (name === undefined) return;
      if (!await screen.confirm(`Use ${name} for the next start?\nHost and agent: ${row.id}\nRevision: ${row.revision}\nThis does not change the current run.`)) return;
      await request('select-config', { name }, row.id, row.revision);
    } },
    ...(['start','stop','restart'] as const).map(action => ({ label: action === 'restart' ? 'Restart — unavailable' : `${action === 'start' ? 'Start' : 'Stop'} selected agent`, disabled: action === 'restart' ? 'Restart can change the local setup. Use Stop. Inspect a recent host report that confirms the agent is stopped. Then use Start. No Restart request is sent.' : eligibility(action), run: async () => {
      const row = snapshot.agents.find(r => r.id === selected); if (!row) return;
      if (!await screen.confirm(`${action === 'start' ? 'Start' : 'Stop'} this agent?\nHost and agent: ${row.id}\nRevision: ${row.revision}`)) return;
      await request(action, undefined, row.id, row.revision);
    } })),
    { label: 'Inspect operations', run: () => request('operations') },
    { label: 'Check operation results', run: () => request('reconcile') },
    { label: 'Sign out', run: () => request('signout') },
    { label: 'Publish profile or instructions', run: () => screen.notice('Publication forms are unavailable. Use the CLI:\nbeehive drafts ~/.beehive/owner\nbeehive tui discover <relay> ~/.beehive/owner\nThis build cannot generate owner keys or list agents independently of hosts.') },
  ] : [
    { label: 'Sign in with saved owner key', run: () => routing('signin') },
    { label: 'Import matching owner key and sign in', run: () => routing('signin', true) },
  ];
  if (scope === 1 && !snapshot.owner) actions.push({ label: 'New private instructions draft…', run: async () => { const name = await screen.input('Private instructions · 1 of 2\nDraft name', '', false, false, v => v.trim() ? '' : 'Enter a draft name.'); if (name === undefined) return; const instructions = await screen.input(`Private instructions · 2 of 2\nDraft: ${name}\nSaved only on this computer. Not published or used by an agent. Do not enter passwords or keys.`, '', false, true, v => v.trim() ? '' : 'Enter instructions.'); if (instructions !== undefined) await request('draft', { name, instructions }); } });
  actions.push({ label: 'Quit Beehive', run: () => screen.close() });
  screen.show(vanished ? [{ id: selected, label: 'Selection no longer available', detail: 'This item is no longer available. Select a current item. No operation will use a replacement item.' }, ...rows] : scope === 0 ? snapshot.local : snapshot.agents.length ? snapshot.agents : [{ id: 'empty', label: snapshot.owner ? 'No host reports yet' : 'Owner sign-in required', detail: 'Sign in as owner to manage agents. This list uses private host reports, not a separate agent directory. An empty list or an offline host does not mean agents are stopped. Local Host and local drafts do not require sign-in.' }], actions, selected);
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
  } catch { screen.notice('Beehive could not read the response. Quit and inspect saved operation records before you try again.'); }
});
input.on('close', () => screen.close());
screen.onCancel = () => { if (pending) output.write(JSON.stringify({ cancel: pending.id }) + '\n'); };
await screen.done;
output.end(JSON.stringify({ quit: true }) + '\n'); input.close();
process.exit(0);
