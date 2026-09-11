import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { hostname } from 'node:os';
import { fork } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { readHostIdentityPublic } from './host-identity.ts';
import { installationPublicSlots } from './slots.ts';
import { createControllerConfig, readControllerConfig } from './controller-config.ts';
import { ownerPublicInput } from './host-setup.ts';
import { managementClient } from './intents.ts';
import { message, type Message } from './protocol.ts';
import { profileDrafts, editProfileDraft } from './profile-drafts.ts';

export type ManagerRequest = { id: number; action: string; values?: Record<string, string>; target?: string; revision?: number };
export type ManagerItem = { id: string; label: string; detail: string; evidence?: string; disabled?: Record<string, string> };
export type ManagerSnapshot = { local: ManagerItem[]; agents: (ManagerItem & { revision: number; configurations: string[] })[]; routing?: { owner: string; relay: string }; owner?: string; status: string };
const short = (s: string) => s.length > 22 ? `${s.slice(0,8)}…${s.slice(-6)}` : s;
const describe = (v: unknown): string => {
  if (v === undefined || v === null) return 'Not reported';
  if (typeof v !== 'object') return typeof v === 'string' && /^[0-9a-f]{32,}$/i.test(v) ? short(v) : String(v);
  if (!Object.keys(v).length) return 'Not reported';
  return Object.entries(v).map(([k, value]) => `${k}: ${describe(value)}`).join('\n');
};

/** One bounded Node helper per explicit credential operation. Completion waits for exit.
 * Cancellation cannot undo OS persistence; callers must inspect before retrying. */
export function managerCredential(input: object, signal: AbortSignal, helper = new URL('./manager-credential-child.ts', import.meta.url)): Promise<any> {
  signal.throwIfAborted();
  return new Promise((resolve, reject) => {
    const child = fork(fileURLToPath(helper), [], {
      execPath: process.execPath, execArgv: [], silent: true,
      env: { PATH: '/usr/bin:/bin', ...(process.env.HOME ? { HOME: process.env.HOME } : {}) },
    });
    let result: any; let failed = false;
    const cancel = () => { failed = true; child.kill('SIGKILL'); };
    const timer = setTimeout(cancel, 10000);
    signal.addEventListener('abort', cancel, { once: true });
    child.stdout?.resume(); child.stderr?.resume();
    child.once('error', () => { failed = true; });
    child.once('message', value => { result = value; });
    child.once('close', code => {
      clearTimeout(timer); signal.removeEventListener('abort', cancel);
      if (code !== 0 || failed || signal.aborted || !result?.ok) reject(Error('Credential operation cancelled, timed out, denied, mismatched or unavailable. Persistence may have completed; inspect before retry. No reset or plaintext fallback.'));
      else resolve(result);
    });
    child.send(input, error => { if (error) cancel(); });
  });
}

/** Domain controller stays on Node. Existing management intent journal owns remote
 * operation identity, CAS, receipts, reconnect and unknown outcomes. */
export class ManagerController {
  private client?: ReturnType<typeof managementClient>;
  private inventory = new Map<string, Message>();
  private offers = new Map<string, Message>();
  private owner?: string;
  private generation = 0;
  private active?: AbortController;
  private closed = false;
  private lastId = 0;
  private status = 'Local Host does not require owner sign-in. Quit/sign out never Stop.';
  readonly home: string;
  readonly changed: (snapshot: ManagerSnapshot) => void;
  private credential: typeof managerCredential;
  private connect: typeof managementClient;
  constructor(home: string, changed: (snapshot: ManagerSnapshot) => void,
    credential = managerCredential, connect = managementClient) {
    this.home = home; this.changed = changed; this.credential = credential; this.connect = connect;
  }
  private get hostDirectory() { return join(this.home, '.beehive', 'host'); }
  private get ownerDirectory() { return join(this.home, '.beehive', 'owner'); }
  snapshot(): ManagerSnapshot {
    const local: ManagerSnapshot['local'] = [];
    try {
      if (existsSync(join(this.hostDirectory, 'host-identity.json'))) {
        const identity = readHostIdentityPublic(this.hostDirectory);
        local.push({ id: 'host', label: identity.pairing.label, evidence: JSON.stringify(identity.pairing, null, 2), detail: `${identity.pairing.label}
Configuration: saved
Relay: ${identity.pairing.relay}
Owner: ${short(identity.pairing.owner)}
Service: not checked by this view

Registration is not Start authority.

Next steps
F2: provision from prepared binding/genesis files.
Quit manager before foreground CLI use:
beehive host --owner-present
Service launch and binding authoring forms are unavailable.` });
        if (existsSync(join(this.hostDirectory, 'setup.json'))) for (const slot of installationPublicSlots(this.hostDirectory)) local.push({ id: slot.agent, label: slot.agent, detail: `Local public slot (not current run status)\nAgent ${slot.agent}\nBindings: ${Object.keys(slot.bindings).join(', ')}` });
      } else local.push({ id: 'missing', label: 'Configure this computer', detail: 'No local host configuration. Save owner PUBLIC npub and relay; host identity is created in OS credentials. No owner sign-in, agent or service Start.' });
    } catch (error) { local.push({ id: 'error', label: 'Retained configuration needs attention', detail: String(error) + '\nPreserved; no automatic reset.' }); }
    const agents: ManagerSnapshot['agents'] = [...this.inventory].map(([id, m]) => {
      const fresh = this.fresh(m);
      const blocked = !fresh ? 'Host stale or unreachable. Refresh before acting.' : this.client?.status().some(o => o.request.host === m.host && o.request.agent === m.agent && !['completed','failed'].includes(o.state)) ? 'Unresolved operation: inspect receipts before acting.' : '';
      return { id, label: `Agent ${short(m.agent)} · ${fresh ? m.body.phase : 'UNKNOWN'}`, revision: m.revision, configurations: Object.keys((m.body.configurations ?? {}) as object),
        disabled: { 'select-config': blocked, stop: blocked, start: blocked || (m.body.phase !== 'stopped' || m.body.actualRun || m.body.assignedHost !== m.host ? 'Requires fresh assigned stopped report without an actual run.' : '') },
        evidence: JSON.stringify(m, null, 2), detail: `Agent ${short(m.agent)}
Host ${short(m.host)}
${fresh ? 'Recent host report' : 'UNKNOWN — stale or unreachable; last reported below'}
Reported phase: ${m.body.phase}
Start authority: ${m.body.assignedHost === m.host ? 'Assigned to this host' : 'Not assigned to this host'}

Actual run (reported)
${m.body.actualRun ? describe(m.body.actualRun) : 'No actual run in this report'}

Selected for next Start
${describe(m.body.selectedNext)}
Choosing next does not change the actual run.` };
    });
    for (const [id, m] of this.offers) agents.push({ id: `host:${id}`, label: `Host ${short(id)}`, revision: m.revision, configurations: [], evidence: JSON.stringify(m, null, 2), detail: `Host availability report
Host ${short(id)}
${this.fresh(m) ? 'Recent host report' : 'UNKNOWN — stale or unreachable'}
${describe(m.body)}
Availability is not agent Start authority.` });
    for (const operation of this.client?.status() ?? []) agents.push({ id: `operation:${operation.request.id}`, label: `${operation.request.type} · ${operation.state} · ${operation.request.id.slice(0,8)}`, revision: operation.request.revision, configurations: [], evidence: JSON.stringify(operation, null, 2), detail: `Operation: ${operation.request.type}
State: ${operation.state}
Host: ${short(operation.request.host)}
Agent: ${short(operation.request.agent)}
Captured revision: ${operation.request.revision}
Publication: ${describe(operation.publication)}
Receipt/result: ${describe(operation.result)}

Publication is not acceptance.
Outcome unknown? Check receipts; do not submit again.
A Stop receipt is not a fresh stopped inventory report.` });
    let routing: ReturnType<typeof readControllerConfig>;
    try { routing = readControllerConfig(this.ownerDirectory); } catch { /* Invalid retained routing is refused by sign-in; never reset here. */ }
    return { local, agents, routing: routing ? { owner: routing.owner, relay: routing.relay } : undefined, owner: this.owner, status: this.status };
  }

  private fresh(m: Message) { return Boolean(this.client?.connected && Date.now() - Number(m.body.observedAt) <= 6000); }
  refresh() { if (!this.closed) this.changed(this.snapshot()); }
  cancel() { this.generation++; this.active?.abort(); }
  close() { this.closed = true; this.cancel(); this.client?.close(); this.client = undefined; this.owner = undefined; }
  async request(request: ManagerRequest) {
    if (this.closed || request.id <= this.lastId || this.active) return;
    this.lastId = request.id;
    const abort = new AbortController(); this.active = abort;
    const generation = ++this.generation;
    const check = () => { abort.signal.throwIfAborted(); if (this.closed || generation !== this.generation) throw Error('Cancelled'); };
    const v = request.values ?? {};
    try {
      if (request.action === 'configure') {
        const owner = ownerPublicInput(v.owner ?? '');
        if (!/^(wss|ws):\/\//.test(v.relay ?? '')) throw Error('Management relay URL required');
        await this.credential({ action: 'configure', directory: this.hostDirectory, label: hostname().slice(0,128), owner, relay: v.relay }, abort.signal);
        check(); this.status = 'Configuration saved. This action did not start a service or agent.';
      } else if (request.action === 'provision') {
        await this.credential({ action: 'provision', directory: this.hostDirectory, binding: v.binding, genesis: v.genesis, secret: v.secret }, abort.signal);
        check(); this.status = 'Local agent provisioned stopped using existing binding/genesis APIs. No Start or relay publication.';
      } else if (request.action === 'signin') {
        const retained = readControllerConfig(this.ownerDirectory);
        const owner = retained?.owner ?? ownerPublicInput(v.owner ?? '');
        const relay = retained?.relay ?? v.relay ?? '';
        if (!/^(wss|ws):\/\//.test(relay)) throw Error('Management relay URL required');
        const result = await this.credential({ action: 'signin', owner, ...(v.secret ? { secret: v.secret.toLowerCase() } : {}) }, abort.signal);
        delete v.secret; check();
        if (result.secret === null) throw Error('No saved owner key. Use Import owner key with the matching 64-hex private key.');
        if (!retained) createControllerConfig(this.ownerDirectory, owner, relay);
        this.client?.close(); this.inventory.clear(); this.offers.clear();
        this.owner = owner;
        const client = this.connect(join(this.ownerDirectory, 'management-intents'), relay, result.secret, m => {
          if (this.client !== client || this.closed) return;
          const map = m.type === 'availability' ? this.offers : m.type === 'inventory' ? this.inventory : undefined;
          if (map) {
            const key = m.type === 'availability' ? m.host : JSON.stringify([m.host, m.agent]);
            const prior = map.get(key);
            if ((prior || map.size < 1000) && (!prior || Number(m.body.observedAt) > Number(prior.body.observedAt))) map.set(key, m);
          }
          this.refresh();
        }, () => this.refresh(), { catalog: { version: 1, owner, relay, registrations: [] } });
        result.secret = undefined; this.client = client;
        abort.signal.addEventListener('abort', () => client.close(), { once: true });
        await client.ready; check(); this.status = 'Owner signed in. Inventory scope: privately advertising hosts and their agent slots, not independent catalog.';
      } else if (request.action === 'signout') {
        this.client?.close(); this.client = undefined; this.owner = undefined; this.inventory.clear(); this.offers.clear();
        this.status = 'Signed out. OS key retained. Hosts and agents were NOT stopped.';
      } else if (request.action === 'draft') {
        await editProfileDraft(profileDrafts(this.ownerDirectory), async prompt => prompt.startsWith('Profile') ? v.name ?? '' : v.instructions ?? '');
        this.status = 'Private instruction draft saved locally. Not published/applied. Resume through beehive drafts ~/.beehive/owner or beehive tui.';
      } else if (request.action === 'reconcile') { this.client?.reconcile(); this.status = 'Querying receipts; not retrying blocked operations.';
      } else if (request.action === 'operations') {
        this.status = this.client?.status().map(o => `${o.request.id} · ${o.request.host}/${o.request.agent} · ${o.request.type} · ${o.state}: ${o.result ?? o.publication}`).join('\n') || 'No recorded operations in this session.';
      } else if (['start', 'stop', 'restart', 'select-config'].includes(request.action)) {
        if (!this.owner || !this.client) throw Error('Owner sign-in required');
        const current = this.inventory.get(request.target ?? '');
        if (!current || !this.fresh(current) || current.revision !== request.revision) throw Error('Selection changed or host stale/unreachable. Refresh; no request sent.');
        if (request.action === 'restart') throw Error('Restart: Not available in this build. Backend Restart can switch bindings; use explicit Stop, inspect stopped receipt, then Start. No request sent.');
        if (request.action === 'start' && (current.body.phase !== 'stopped' || current.body.actualRun || current.body.assignedHost !== current.host)) throw Error('Start requires a fresh assigned stopped report without an actual run.');
        if (this.client.status().some(o => o.request.host === current.host && o.request.agent === current.agent && !['completed','failed'].includes(o.state))) throw Error('Unresolved operation exists. Inspect operations and reconcile before another action.');
        const operation = message(request.action === 'select-config' ? 'save' : request.action as 'start' | 'stop', current.host, current.agent, current.revision, request.action === 'select-config' ? { configurationAction: 'select', name: v.name } : {});
        this.client.submit(operation);
        this.status = `Operation ${operation.id} · ${current.host}/${current.agent}: durably pending ${request.action}. Publication is not acceptance. Inspect operations; unknown is not stopped.`;
      } else if (request.action !== 'refresh') throw Error('Not available in this build');
    } catch (error) { if (!this.closed) this.status = error instanceof Error ? error.message : 'Operation failed; inspect before retry.'; }
    finally { delete v.secret; this.active = undefined; this.refresh(); }
  }
}
