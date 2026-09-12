import { addDatabricks, databricksNative } from './databricks.ts';
import { existsSync } from 'node:fs';
import { readSettings, saveSettings, settingsId, type Settings, type RegisteredAgent } from './settings.ts';
import { agentNsec } from './settings-credentials.ts';
import { publicKey } from './protocol.ts';
import { fetchAgentProfile } from './agent-profile.ts';
import { serviceStatus, startService, stopService, type ServiceStatus } from './host-service.ts';
import { discoverHarnesses, type DetectedHarness } from './harness-discovery.ts';
import { join } from 'node:path';
import { hostname } from 'node:os';
import { managerCredential } from './manager-credential.ts';
export { managerCredential } from './manager-credential.ts';
import { readHostIdentityPublic } from './host-identity.ts';
import { createControllerConfig, readControllerConfig } from './controller-config.ts';
import { ownerPublicInput } from './host-setup.ts';
import { managementClient } from './intents.ts';
import { fetchRelayName } from './relay-name.ts';
import { message, type Message } from './protocol.ts';
import { profileDrafts, editProfileDraft } from './profile-drafts.ts';

export type ManagerRequest = { id: number; action: string; values?: Record<string, string>; target?: string; revision?: number };
export type ManagerItem = { id: string; label: string; detail: string; evidence?: string; disabled?: Record<string, string> };
export type ManagerSnapshot = { local: ManagerItem[]; agents: (ManagerItem & { revision: number; configurations: string[] })[]; routing?: { owner: string; relay: string }; owner?: string; status: string; settings?: Settings; service?: ServiceStatus; hostRelay?: string; relayName?: string; profilePreview?: RegisteredAgent; models?: string[]; runtimeExecutable?: string; harnesses?: DetectedHarness[]; databricksHost?: string };
const short = (s: string) => s.length > 22 ? `${s.slice(0,8)}…${s.slice(-6)}` : s;

/** Final plain display text thrown by this boundary; never rewrapped or retranslated. */
class PlainStatus extends Error {}
const plain = (text: string) => new PlainStatus(text);
/** Shared backend validation/protocol sentinels are translated here, at the manager
 * display boundary only. Legacy CLI text, tests and result sentinels stay exact;
 * unknown diagnostics are retained after a plain failure context, never replaced. */
const backendMessages: Record<string, string> = {
  'Unsupported Beehive controller configuration': 'Saved Beehive configuration uses an unsupported version.',
  'Invalid retained management relay URL': 'The saved relay URL is invalid.',
  'Controller already configured; use Settings for deliberate changes': 'Owner and relay are already configured. They cannot be changed in this view.',
  'Management relay requires wss:// (ws://127.0.0.1 for fixtures)': 'Enter a relay URL that starts with ws:// or wss://.',
  'Owner PUBLIC npub or 64-character hex key required; never enter a private key': 'Enter the owner’s public key as an npub or 64 hex characters. Do not enter a private key.',
  'Legacy identity retained; explicit consent required for migration': 'The saved host identity uses an older format. It has not been changed. Migration requires explicit permission.',
  'Invalid host credential reference': 'The saved host key reference is invalid.',
  'Explicit stopped migrate-slots required': 'This installation needs migration. Stop its agents before you use migrate-slots.',
  'Installation requires 1..32 slots': 'The installation must contain 1 to 32 agent records.',
  'Invalid slot identity': 'An agent record has an invalid identity.',
  'Invalid slot binding': 'An agent record refers to an invalid local setup.',
  'Public slot binding mismatch': 'The saved agent record does not match its host, owner or agent identity.',
  'Shared harness inventory must not copy key material': 'Shared local setups must not contain identity keys.',
  'Binding cannot carry identity': 'A local setup must not contain host or agent identity fields.',
  'Binding cannot change conversation authority': 'A local setup cannot change permission to use the conversation.',
  'Invalid retired binding definition': 'A retired local setup does not match its saved definition.',
  'Public credential installation required; no automatic plaintext migration': 'This installation must use secure key references. Plain-text keys will not be migrated automatically.',
  'Invalid agent credential reference': 'The saved agent key reference is invalid.',
  'Draft directory must be owner-only': 'Only the local user may access the draft folder.',
  'Draft storage exceeds limit': 'The draft file exceeds the size limit.',
  'Invalid draft storage': 'The saved draft file is invalid.',
  'Invalid draft identity': 'A saved draft has an invalid or duplicate ID.',
  'Draft changed; resume again': 'The draft changed. Open it again before you edit.',
  'Draft storage full; discard unused drafts': 'Draft storage is full. Discard unused drafts through beehive drafts.',
  'Invalid profile name/parent': 'The instructions name or previous revision is invalid. Do not use default as the name.',
  'Invalid nonsecret instructions': 'Enter instructions of no more than 2048 UTF-8 bytes. Do not use control characters.',
  'Profile revision conflict': 'The instructions do not match their revision.',
  'Management journal directory must be owner-only': 'Only the local user may access the operation records folder.',
  'Management journal full': 'Operation storage is full.',
  'Management journal full; no operation submitted': 'Operation storage is full. No operation was submitted.',
  'Oversized intent': 'A saved operation request exceeds the size limit.',
  'Oversized receipt': 'A saved host result exceeds the size limit.',
  'Oversized blocked intent': 'A saved blocked request exceeds the size limit.',
  'Wrong journal scope': 'Saved operation records do not match this owner and relay.',
  'Invalid intent': 'A saved operation request is invalid.',
  'Unmatched journal receipt': 'A saved host result does not match its operation.',
  'Invalid blocked intent': 'A saved blocked request is invalid.',
  'Unmatched durable receipt': 'A saved host result does not match its operation.',
  'Operation ID already prepared': 'This operation ID is already saved.',
  'UI closed': 'Beehive is closed.',
  'Relay disconnected; result unknown': 'The relay disconnected. The operation result is unknown.',
};
const plainBackendMessage = (message: string) => backendMessages[message] ?? `Could not complete the operation: ${message}`;

/** Raw management values keep their exact protocol/evidence bytes; only the default
 * summary gets plain labels. Unknown nested fields stay technical, never discarded. */
const plainValues: Record<string, string> = {
  'immutable publication observed; no agent application': 'Private instructions published. Not applied to an agent.',
  'host terminal receipt': 'Host result received.',
  'relay policy failure; automatic retry disabled; reconcile, then retry after policy repair': 'Relay policy blocked the operation. Automatic retry is disabled. Check operation results. Correct the policy problem before you retry through the CLI.',
  'relay observed; not host admission': 'Relay publication confirmed. Host acceptance not confirmed.',
  'unconfirmed': 'Not confirmed.',
  'accepted': 'Accepted by host.',
  'saved; running configuration unchanged': 'Configuration saved. Current run unchanged.',
  'interrupted-reconcile-locally': 'Operation interrupted. Result unknown. Inspect and repair the saved state on the host before you try again.',
};
const plainStates: Record<string, string> = { pending: 'Pending', completed: 'Completed', failed: 'Failed', unknown: 'Unknown' };
const plainTypes: Record<string, string> = { start: 'Start', stop: 'Stop', restart: 'Restart', move: 'Move', metadata: 'Publish profile', profile: 'Publish instructions' };
const plainOperationType = (request: Message) => request.type === 'save' && request.body.configurationAction === 'select' ? 'Choose configuration' : plainTypes[request.type] ?? request.type;
const operationText = (value: unknown) => typeof value === 'string' ? plainValues[value] ?? value : describe(value);
/** Named launch choice: the model/workspace/instruction selection of one agent. */
const selectionLabels: Record<string, string> = {
  'model': 'Model', 'workspace': 'Workspace', 'profile': 'Instructions revision',
  'harnessSetup.id': 'Local setup', 'harnessSetup.fingerprint': 'Local setup fingerprint',
  'configuration.name': 'Configuration name', 'configuration.revision': 'Configuration revision',
  'behavior.name': 'Instructions name', 'behavior.instructions': 'Private instructions',
  'behavior.revision': 'Instructions revision', 'behavior.parent': 'Previous instructions revision',
};
const runLabels: Record<string, string> = { ...selectionLabels, ...Object.fromEntries(Object.entries(selectionLabels).map(([key, value]) => [`selection.${key}`, value])) };
/** Host availability pairing: host infrastructure, not agent configuration. */
const availabilityLabels: Record<string, string> = {
  'configuration': 'Host configuration', 'configuration.host': 'Host', 'configuration.owner': 'Owner',
  'configuration.relay': 'Relay', 'configuration.label': 'Host name', 'configuration.observedAt': 'Report time', 'observedAt': 'Report time',
};
/** These containers hold only labelled choice fields; their children speak alone. */
const inlineChoice = new Set(['harnessSetup', 'configuration', 'behavior', 'selection', 'selection.harnessSetup', 'selection.configuration', 'selection.behavior']);
const describe = (v: unknown, labels: Record<string, string> = {}, path = ''): string => {
  if (v === undefined || v === null) return 'Not reported';
  if (typeof v !== 'object') {
    if (typeof v !== 'string') return String(v);
    if ((path === 'profile' || path.endsWith('.profile')) && v === 'default') return 'Default instructions';
    return /^[0-9a-f]{32,}$/i.test(v) ? short(v) : v;
  }
  if (!Object.keys(v).length) return 'Not reported';
  return Object.entries(v).map(([k, value]) => {
    const child = path ? `${path}.${k}` : k;
    const rendered = describe(value, labels, child);
    if (labels !== availabilityLabels && inlineChoice.has(child) && value !== null && !Array.isArray(value) && typeof value === 'object') return rendered;
    return `${labels[child] ?? k}: ${rendered}`;
  }).join('\n');
};

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
  private service: ServiceStatus = { state: 'unknown' };
  private probing = false;
  private profilePreview?: RegisteredAgent;
  private models?: string[];
  private runtimeExecutable?: string;
  private harnesses?: DetectedHarness[];
  private databricksHost?: string;
  private lastId = 0;
  private relayName?: { relay: string; name?: string };
  private relayNamePending?: { relay: string; abort: AbortController };
  private status = 'Local Host does not require owner sign-in. Quitting or signing out does not stop hosts or agents.';
  readonly home: string;
  readonly changed: (snapshot: ManagerSnapshot) => void;
  private credential: typeof managerCredential;
  private connect: typeof managementClient;
  private fetchName: typeof fetchRelayName;
  constructor(home: string, changed: (snapshot: ManagerSnapshot) => void,
    credential = managerCredential, connect = managementClient, fetchName = fetchRelayName) {
    this.home = home; this.changed = changed; this.credential = credential; this.connect = connect; this.fetchName = fetchName;
  }
  private get hostDirectory() { return join(this.home, '.beehive', 'host'); }
  private get ownerDirectory() { return join(this.home, '.beehive', 'owner'); }
  snapshot(): ManagerSnapshot {
    const local: ManagerSnapshot['local'] = [];
    try {
      if (existsSync(join(this.hostDirectory, 'host-identity.json'))) {
        const identity = readHostIdentityPublic(this.hostDirectory);
        local.push({ id: 'host', label: identity.pairing.label, detail: 'Host configured. Agent registration is not execution authority.' });
      } else local.push({ id: 'missing', label: 'Configure this computer', detail: 'No local host configuration. Save the owner’s public key and relay URL. Beehive creates a host identity in the secure credential store. No owner sign-in is needed. This does not create an agent or start a host.' });
    } catch (error) { local.push({ id: 'error', label: 'Saved configuration needs attention', detail: (error instanceof Error ? backendMessages[error.message] ?? `Could not read saved configuration: ${error.message}` : `Could not read saved configuration: ${String(error)}`) + '\nSaved data has not been reset.' }); }
    const agents: ManagerSnapshot['agents'] = [...this.inventory].map(([id, m]) => {
      const fresh = this.fresh(m);
      const blocked = !fresh ? 'The host report is old or the host cannot be reached. Wait for a recent report before you act.' : this.client?.status().some(o => o.request.host === m.host && o.request.agent === m.agent && !['completed','failed'].includes(o.state)) ? 'An operation has no confirmed result. Inspect operations before you act.' : '';
      return { id, label: `Agent ${short(m.agent)} · ${fresh ? m.body.phase : 'Unknown'}`, revision: m.revision, configurations: Object.keys((m.body.configurations ?? {}) as object),
        disabled: { 'select-config': blocked, stop: blocked, start: blocked || (m.body.phase !== 'stopped' || m.body.actualRun || m.body.assignedHost !== m.host ? 'Start requires a recent report from the assigned host. It must report the agent stopped with no current run.' : '') },
        evidence: JSON.stringify(m, null, 2), detail: `Agent ${short(m.agent)}
Host ${short(m.host)}
${fresh ? 'Recent host report' : 'Current status unknown. The report is old or the host cannot be reached. Last report:'}
Reported status: ${m.body.phase}
Assigned host: ${m.body.assignedHost === m.host ? 'Assigned to this host' : 'Not assigned to this host'}

Current run (host report)
${m.body.actualRun ? describe(m.body.actualRun, runLabels) : 'No current run in this report'}

Configuration for next start
${describe(m.body.selectedNext, selectionLabels)}
This choice does not change the current run.` };
    });
    for (const [id, m] of this.offers) agents.push({ id: `host:${id}`, label: `Host ${short(id)}`, revision: m.revision, configurations: [], evidence: JSON.stringify(m, null, 2), detail: `Host report
Host ${short(id)}
${this.fresh(m) ? 'Recent host report' : 'Current status unknown. The report is old or the host cannot be reached.'}
${describe(m.body, availabilityLabels)}
A host report does not grant permission to start an agent.` });
    for (const operation of this.client?.status() ?? []) agents.push({ id: `operation:${operation.request.id}`, label: `${plainOperationType(operation.request)} · ${plainStates[operation.state] ?? operation.state} · ${operation.request.id.slice(0,8)}`, revision: operation.request.revision, configurations: [], evidence: JSON.stringify(operation, null, 2), detail: `Operation: ${plainOperationType(operation.request)}
State: ${plainStates[operation.state] ?? operation.state}
Host: ${short(operation.request.host)}
Agent: ${short(operation.request.agent)}
Revision when requested: ${operation.request.revision}
Relay status: ${operationText(operation.publication)}
${operation.request.type === 'profile' ? 'Result' : 'Host result'}: ${operationText(operation.result)}

Relay publication does not confirm that the host accepted the operation.
If the result is unknown, check operation results before you submit again.
A Stop result is not a recent host report that confirms the agent is stopped.` });
    let routing: ReturnType<typeof readControllerConfig>;
    try { routing = readControllerConfig(this.ownerDirectory); } catch { /* Invalid retained routing is refused by sign-in; never reset here. */ }
    let settings: Settings | undefined, hostRelay: string | undefined;
    try { settings = readSettings(this.hostDirectory); if (existsSync(join(this.hostDirectory,'host-identity.json'))) hostRelay = readHostIdentityPublic(this.hostDirectory).pairing.relay; } catch { /* Existing error row remains actionable; never reset settings. */ }
    const footerRelay = hostRelay ?? routing?.relay;
    this.observeRelayName(footerRelay);
    return { settings, service: this.service, hostRelay, relayName: this.relayName && this.relayName.relay === footerRelay ? this.relayName.name : undefined, profilePreview: this.profilePreview, models: this.models, runtimeExecutable: this.runtimeExecutable, harnesses: this.harnesses, databricksHost: this.databricksHost, local, agents, routing: routing ? { owner: routing.owner, relay: routing.relay } : undefined, owner: this.owner, status: this.status };
  }

  private fresh(m: Message) { return Boolean(this.client?.connected && Date.now() - Number(m.body.observedAt) <= 6000); }
  /** Optional bounded footer relay name. One attempt per configured relay URL; absence,
   * failure or cancellation only omits the name. It never changes the reported
   * connection state, blocks the UI, resets configuration or invents a name, and a
   * late or stale completion is never applied to a different relay. */
  private observeRelayName(relay: string | undefined) {
    if (this.closed) return;
    if (this.relayName?.relay === relay || this.relayNamePending?.relay === relay) return;
    this.relayNamePending?.abort.abort();
    this.relayNamePending = undefined;
    this.relayName = undefined;
    if (!relay) return;
    const pending = { relay, abort: new AbortController() };
    this.relayNamePending = pending;
    void Promise.resolve().then(() => this.fetchName(relay, pending.abort.signal)).catch(() => undefined).then(name => {
      if (this.closed || this.relayNamePending !== pending) return;
      this.relayNamePending = undefined;
      this.relayName = { relay, name };
      if (!this.closed) this.changed(this.snapshot());
    });
  }
  refresh() {
    if (this.closed) return;
    this.changed(this.snapshot());
    if (!this.probing) {
      this.probing = true;
      void serviceStatus(this.hostDirectory).then(status => { this.service = status; if (!this.closed) this.changed(this.snapshot()); }).finally(() => { this.probing = false; });
    }
  }
  cancel() { this.generation++; this.active?.abort(); }
  close() { this.closed = true; this.cancel(); this.relayNamePending?.abort.abort(); this.client?.close(); this.client = undefined; this.owner = undefined; }
  async request(request: ManagerRequest) {
    if (this.closed || request.id <= this.lastId || this.active) return;
    this.lastId = request.id;
    const abort = new AbortController(); this.active = abort;
    const generation = ++this.generation;
    const check = () => { abort.signal.throwIfAborted(); if (this.closed || generation !== this.generation) throw plain('Stopped waiting. Changes may already be saved or submitted. Inspect before you try again.'); };
    const v = request.values ?? {};
    try {
      if (request.action === 'profile-preview') {
        const identity = readHostIdentityPublic(this.hostDirectory);
        const key = publicKey(agentNsec(v.secret ?? ''));
        this.profilePreview = undefined;
        const profile = await fetchAgentProfile(identity.pairing.relay,key,abort.signal); check();
        this.profilePreview = { publicKey: key, key: { service: 'beehive', role: 'agent', publicKey: key }, ...profile };
        this.status = profile.profileState === 'found' ? 'Signed public profile found.' : profile.profileState === 'none' ? 'No profile found. You can register with the public key.' : 'Profile lookup unavailable. You can register without a profile.';
      } else if (request.action === 'register-agent') {
        const key = publicKey(agentNsec(v.secret ?? ''));
        if (this.profilePreview?.publicKey !== key) throw plain('Confirm the public key first.');
        const { profile, profileState } = this.profilePreview;
        await this.credential({ action: 'register-agent', directory: this.hostDirectory, secret: v.secret, profile: { profile, profileState } },abort.signal); check();
        this.profilePreview = undefined; this.status = 'Agent registered. Not assigned or started.';
      } else if (request.action === 'provider-form') {
        this.databricksHost = process.env.DATABRICKS_HOST ?? ''; this.status = 'Provider credentials stay in Beehive’s OS store.';
      } else if (request.action === 'add-databricks') {
        await addDatabricks(this.hostDirectory,v.name,v.endpoint,abort.signal); check(); this.status = 'Databricks signed in and saved. No agent was started.';
      } else if (request.action === 'add-openai') {
        await this.credential({ action: 'add-openai', directory: this.hostDirectory, name: v.name, secret: v.secret },abort.signal); check(); this.status = 'Provider saved. No agent was started.';
      } else if (request.action === 'runtime-form') {
        this.runtimeExecutable = undefined; this.harnesses = undefined; this.models = undefined;
        const harnesses = await discoverHarnesses(abort.signal); check(); this.harnesses = harnesses;
        this.runtimeExecutable = this.harnesses.find(h => h.id === 'buzz-agent' && h.providers.length)?.executable; this.models = undefined;
        this.status = this.harnesses.map(h => `${h.label}: ${h.reason}`).join(' · ');
      } else if (request.action === 'models') {
        this.models = undefined;
        const provider = readSettings(this.hostDirectory).providers.find(p => p.id === v.provider);
        const result = provider?.type === 'databricks_v2' ? await databricksNative({ action: 'models',host:provider.endpoint,key:provider.key },abort.signal) : await this.credential({ action: 'models', directory: this.hostDirectory, provider: v.provider },abort.signal); check(); this.models = result.models; this.status = 'Model list loaded. Custom model is also available.';
      } else if (request.action === 'add-runtime') {
        const selected = this.harnesses?.find(h => h.id === (v.harness ?? 'buzz-agent'));
        const previous = readSettings(this.hostDirectory);
        const provider = previous.providers.find(p => p.id === v.provider);
        if (!selected?.executable || !provider || !selected.providers.includes(provider.type) || !['buzz-agent','codex'].includes(selected.id)) throw plain('No supported executable/provider combination found.');
        saveSettings(this.hostDirectory,{ ...previous, runtimes: [...previous.runtimes,{ id: settingsId(), name: v.name ?? '', harness: selected.id as 'buzz-agent' | 'codex', executable: selected.executable, ...(selected.id === 'codex' ? { cli: selected.cli } : {}), providerId: v.provider ?? '', model: v.model ?? '', ...(v.effort ? { effort: v.effort } : {}) }] },previous.revision);
        this.status = 'Runtime saved for new runs. Running agents did not change. Host loading is reported separately.';
      } else if (request.action === 'host-start') {
        this.service = await startService(this.hostDirectory); check(); this.status = 'Host running. Registration and settings do not start agents.';
      } else if (request.action === 'host-stop') {
        if (!v.instance) throw plain('A verified host instance is required.');
        this.service = await stopService(this.hostDirectory,v.instance); check(); this.status = this.service.state === 'stopped' ? 'Host stopped. Owned teardown completed.' : 'Host teardown is unconfirmed. Status unknown.';
      } else if (request.action === 'configure') {
        const owner = ownerPublicInput(v.owner ?? '');
        if (!/^(wss|ws):\/\//.test(v.relay ?? '')) throw plain('Enter a relay URL that starts with ws:// or wss://.');
        await this.credential({ action: 'configure', directory: this.hostDirectory, label: hostname().slice(0,128), owner, relay: v.relay }, abort.signal);
        check(); this.status = 'Configuration saved. This action did not start a service or agent.';
      } else if (request.action === 'provision') {
        await this.credential({ action: 'provision', directory: this.hostDirectory, binding: v.binding, genesis: v.genesis, secret: v.secret }, abort.signal);
        check(); this.status = 'Local agent added and stopped. Nothing was started or published to the relay.';
      } else if (request.action === 'signin') {
        const retained = readControllerConfig(this.ownerDirectory);
        const owner = retained?.owner ?? ownerPublicInput(v.owner ?? '');
        const relay = retained?.relay ?? v.relay ?? '';
        if (!/^(wss|ws):\/\//.test(relay)) throw plain('Enter a relay URL that starts with ws:// or wss://.');
        const result = await this.credential({ action: 'signin', owner, ...(v.secret ? { secret: v.secret.toLowerCase() } : {}) }, abort.signal);
        delete v.secret; check();
        if (result.secret === null) throw plain('No saved owner key. Choose Import matching owner key and sign in. Enter the matching private key with 64 hex characters.');
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
        await client.ready; check(); this.status = 'Owner signed in. This list shows private host reports and their agents. It is not a separate agent directory.';
      } else if (request.action === 'signout') {
        this.client?.close(); this.client = undefined; this.owner = undefined; this.inventory.clear(); this.offers.clear();
        this.status = 'Signed out. The owner key is still saved on this computer. Hosts and agents were not stopped.';
      } else if (request.action === 'draft') {
        await editProfileDraft(profileDrafts(this.ownerDirectory), async prompt => prompt.startsWith('Profile') ? v.name ?? '' : v.instructions ?? '');
        this.status = 'Private instructions draft saved on this computer. Not published or used by an agent. To resume, use beehive drafts ~/.beehive/owner or beehive tui.';
      } else if (request.action === 'reconcile') { this.client?.reconcile(); this.status = 'Checking operation results. Operations blocked by relay policy will not be retried.';
      } else if (request.action === 'operations') {
        this.status = this.client?.status().map(o => `${o.request.id} · ${o.request.host}/${o.request.agent} · ${plainOperationType(o.request)} · ${plainStates[o.state] ?? o.state}: ${operationText(o.result ?? o.publication)}`).join('\n') || 'No saved operations.';
      } else if (['start', 'stop', 'restart', 'select-config'].includes(request.action)) {
        if (!this.owner || !this.client) throw plain('Owner sign-in required');
        const current = this.inventory.get(request.target ?? '');
        if (!current || !this.fresh(current) || current.revision !== request.revision) throw plain('The selection changed, or the host report is old or unavailable. Select an agent with a recent report. No request was sent.');
        if (request.action === 'restart') throw plain('Restart can change the local setup. Use Stop. Inspect a recent host report that confirms the agent is stopped. Then use Start. No Restart request is sent. No request was sent.');
        if (request.action === 'start' && (current.body.phase !== 'stopped' || current.body.actualRun || current.body.assignedHost !== current.host)) throw plain('Start requires a recent report from the assigned host. It must report the agent stopped with no current run.');
        if (this.client.status().some(o => o.request.host === current.host && o.request.agent === current.agent && !['completed','failed'].includes(o.state))) throw plain('An operation has no confirmed result. Inspect operations and check operation results before you act again.');
        const operation = message(request.action === 'select-config' ? 'save' : request.action as 'start' | 'stop', current.host, current.agent, current.revision, request.action === 'select-config' ? { configurationAction: 'select', name: v.name } : {});
        this.client.submit(operation);
        this.status = `Operation ${operation.id}: ${plainOperationType(operation)} saved and pending.\nHost: ${current.host}\nAgent: ${current.agent}\nRelay publication does not confirm host acceptance. Inspect operations. Unknown does not mean stopped.`;
      } else if (request.action !== 'refresh') throw plain('Not available in this build');
    } catch (error) { if (!this.closed) this.status = error instanceof PlainStatus ? error.message : abort.signal.aborted && error === abort.signal.reason ? 'Stopped waiting. Changes may already be saved or submitted. Inspect before you try again.' : error instanceof Error ? plainBackendMessage(error.message) : `Could not complete the operation: ${String(error)}`; }
    finally { delete v.secret; this.active = undefined; this.refresh(); }
  }
}
