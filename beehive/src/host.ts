import { accessSync, constants, existsSync, mkdirSync, rmdirSync, realpathSync, statSync, readFileSync } from 'node:fs';
import { join, isAbsolute } from 'node:path';
import { createHash } from 'node:crypto';
import { connect, validateRelayURL } from './client.ts';
import { digest, fields, message, object, publicKey, text, type Message } from './protocol.ts';
import { readPrivate, writePrivate } from './storage.ts';
import { ConversationSession } from './broker.ts';
import { spawnOwned, type OwnedProcess } from './owned.ts';
import { AgentSession, prepareAgent, type AgentLaunch, type Catalog, type Evidence } from './acp.ts';
import { prepareConversation, conversationSummary, type ConversationSetup } from './conversation.ts';

export type Setup = { host: string; ownerSecret: string; agentSecret: string; runner: string; args: string[]; workspace: string; mode: 'fixture' | 'buzz-agent-databricks-v2'; databricksHost?: string; serviceHome?: string; configDirectory?: string; conversation?: ConversationSetup };
type Selection = { model: string; workspace: string; profile: string };
type State = { binding: { host: string; owner: string; agent: string }; revision: number; phase: 'stopped' | 'transitioning' | 'running' | 'quarantined'; selected: Selection; actual: null | { selection: Selection; executableHash: string; run: string; evidence?: Evidence }; operations: Record<string, { fingerprint: string; reply: Message }>; outbox: Message[] };
export function validateSetup(value: unknown): Setup {
  const s = object(value);
  for (const key of ['host','ownerSecret','agentSecret','runner','workspace']) text(s[key]);
  publicKey(String(s.ownerSecret)); publicKey(String(s.agentSecret));
  if (!isAbsolute(String(s.runner)) || !isAbsolute(String(s.workspace)) || !Array.isArray(s.args) || s.args.some(a => typeof a !== 'string') || !['fixture','buzz-agent-databricks-v2'].includes(String(s.mode))) throw Error('Invalid local setup');
  return s as Setup;
}
/** Fixed-authority first slice. Keys never constitute a remote assignment grant. */
export async function host(directory: string, url: string) {
  validateRelayURL(url);
  const setup = validateSetup(readPrivate(join(directory,'setup.json')));
  const agent = publicKey(setup.agentSecret);
  const lock = join(directory,'host.lock');
  const path = join(directory,'journal.json');
  let state: State = existsSync(path) ? readPrivate(path) as State : {
    binding: { host: setup.host, owner: publicKey(setup.ownerSecret), agent }, revision: 0, phase: 'stopped', selected: { model: setup.mode === 'fixture' ? 'fixture-model' : 'databricks-claude-haiku-4-5', workspace: setup.workspace, profile: 'default' }, actual: null, operations: {}, outbox: [],
  };
  if (state.binding.host !== setup.host || state.binding.owner !== publicKey(setup.ownerSecret) || state.binding.agent !== agent) throw Error('Saved ownership/assignment mismatch');
  mkdirSync(lock, { mode: 0o700 }); // No PID-only stale-lock recovery.
  if (state.phase !== 'stopped') state.phase = 'quarantined';
  const save = () => writePrivate(path,state);
  try { save(); } catch (e) { rmdirSync(lock); throw e; }
  let owned: OwnedProcess | ConversationSession | undefined;
  let acp: AgentSession | ConversationSession | undefined;
  let catalog: Catalog | { state: 'not-probed' | 'failed'; authentication: 'unverified' } = { state: 'not-probed', authentication: 'unverified' };
  function inventory() {
    return message('inventory',setup.host,agent,state.revision, {
      assignedHost: setup.host, phase: state.phase, selectedNext: state.selected, actualRun: state.actual,
      ...(setup.conversation ? { conversation: conversationSummary(setup.conversation) } : {}),
      catalog, setup: setup.mode, models: [setup.mode === 'fixture' ? 'fixture-model' : 'databricks-claude-haiku-4-5'],
      workspaces: [setup.workspace], profiles: ['default'], readiness: setup.conversation ? (state.actual?.evidence ? 'Conversation session model acknowledged and response completed; relay delivery unverified' : 'Waiting for an admitted conversation and local provider sign-in; no synthetic prompt') : setup.mode === 'fixture' ? 'fixture-only' : (state.phase === 'running' && state.actual?.evidence ? 'ACP model acknowledged + same-session response (not provider attestation or Buzz relay agent)' : 'unverified: Start runs an ACP greeting probe; sign in locally as host service user if required'), observedAt: Date.now(),
    });
  }
  let closing = false;
  let queue = Promise.resolve();
  let initialized = false;
  let client: ReturnType<typeof connect>;
  try { client = connect(url,setup.ownerSecret,m => {
    if (!initialized) return;
    // Cancellation is signalled outside the serialized mutation queue. Admission is
    // still exact-authority/revision and never bypasses durable receipt processing.
    if (!closing && m.type === 'stop' && m.host === setup.host && m.agent === agent && m.revision === state.revision && state.phase === 'transitioning' && state.actual && !Object.hasOwn(state.operations, m.id) && Object.keys(m.body).length === 0) acp?.cancel();
    queue = queue.then(() => handle(m)).catch(() => { state.phase = 'quarantined'; save(); });
  }, () => {
    if (closing || !initialized) return;
    // Replay committed receipts after transport recovery. Incoming relay history
    // still traverses the fingerprint/revision fence; reconnect cannot repeat effects.
    for (const m of state.outbox) publish(m);
    publish(inventory());
  }); }
  catch (e) { rmdirSync(lock); throw e; }
  const publish = (m: Message) => { try { client.send(m); } catch { /* Durable outbox retained for reconnect/restart. */ } };
  async function stopOwned() {
    if (!owned) throw Error('Unknown ownership; quarantined');
    await owned.stop(); owned = undefined; acp = undefined;
  }
  async function handle(m: Message) {
    if (closing || m.host !== setup.host || !['inspect','save','start','stop'].includes(m.type)) return;
    if (m.type === 'inspect') { publish(inventory()); return; }
    const fingerprint = digest(JSON.stringify(m)).toString('hex');
    const previous = Object.hasOwn(state.operations,m.id) ? state.operations[m.id] : undefined;
    if (previous) {
      publish(previous.fingerprint === fingerprint ? previous.reply : message('receipt',setup.host,m.agent,state.revision,{ operation: m.id, result: 'operation-id-conflict' })); return;
    }
    let result = 'accepted';
    if (m.agent !== agent) result = 'not-authority';
    else if (m.revision !== state.revision) result = 'revision-conflict';
    else if ((state.phase === 'quarantined' || state.phase === 'transitioning') && !(m.type === 'stop' && (owned instanceof ConversationSession ? owned.connected : owned?.child.connected))) result = 'quarantined';
    else {
      // Reserve operation before any spawn/kill; interrupted admission never retries effects.
      const pending = message('receipt',setup.host,agent,state.revision,{ operation: m.id, result: 'interrupted-reconcile-locally' });
      state.operations[m.id] = { fingerprint, reply: pending }; save();
      try {
        if (m.type === 'save') {
          fields(m.body,['model','workspace','profile']);
          const selected = { model: text(m.body.model), workspace: text(m.body.workspace), profile: text(m.body.profile) };
          if (selected.model !== (setup.mode === 'fixture' ? 'fixture-model' : 'databricks-claude-haiku-4-5') || selected.workspace !== setup.workspace || selected.profile !== 'default') throw Error('Unsupported selection');
          state.selected = selected; result = 'saved; running configuration unchanged';
        } else if (m.type === 'start') {
          fields(m.body,[]);
          if (state.phase !== 'stopped') throw Error('Already running');
          accessSync(setup.runner,constants.X_OK);
          if (realpathSync(setup.workspace) !== setup.workspace || !statSync(setup.workspace).isDirectory()) throw Error('Workspace changed');
          const launch: AgentLaunch | undefined = setup.mode === 'fixture' ? undefined : { executable: setup.runner, args: setup.args, workspace: state.selected.workspace, home: text(setup.serviceHome), configDirectory: text(setup.configDirectory), databricksHost: text(setup.databricksHost), model: state.selected.model };
          if (launch) prepareAgent(launch); // Prerequisite failure preserves stopped state.
          if (setup.conversation) {
            if (!launch) throw Error('Conversation requires a Buzz Agent harness setup');
            prepareConversation(setup.conversation, launch, setup.agentSecret, state.binding.owner);
          }
          state.actual = { selection: structuredClone(state.selected), executableHash: createHash('sha256').update(readFileSync(setup.runner)).digest('hex'), run: m.id };
          state.phase = 'transitioning'; save();
          if (setup.mode === 'fixture') {
            owned = spawnOwned(setup.runner, setup.args, state.actual.selection.workspace, { PATH: '/usr/bin:/bin', BEEHIVE_FIXTURE: '1' });
            owned.child.stdout.resume(); owned.child.stderr.resume();
            await owned.ready;
          } else if (setup.conversation) {
            const session = new ConversationSession(setup.conversation, launch!, setup.agentSecret, state.binding.owner);
            acp = session; owned = session;
            try {
              state.actual.evidence = await session.verify();
              if (!session.healthy) throw Error('Conversation failed before running commit');
            } catch (e) {
              await stopOwned(); state.phase = 'stopped'; state.actual = null;
              throw e;
            }
          } else {
            const session = new AgentSession(launch!); acp = session;
            owned = session.owned;
            try {
              catalog = await session.catalog();
              state.actual.evidence = await session.verify();
              if (!session.healthy) throw Error('ACP session failed before running commit');
            } catch (e) {
              catalog = { state: 'failed', authentication: 'unverified' };
              await stopOwned(); state.phase = 'stopped'; state.actual = null;
              throw e;
            }
          }
          state.phase = 'running';
        } else {
          fields(m.body,[]);
          if (state.phase !== 'stopped') {
            state.phase = 'transitioning'; save(); await stopOwned(); state.phase = 'stopped'; state.actual = null;
          }
        }
        state.revision++;
      } catch (error) {
        if (state.phase === 'transitioning') state.phase = 'quarantined';
        result = error instanceof Error ? error.message : 'Operation failed';
      }
    }
    const reply = message('receipt',setup.host,m.agent,state.revision,{ operation: m.id, result });
    Object.defineProperty(state.operations,m.id,{ value: { fingerprint, reply }, enumerable: true, configurable: true, writable: true });
    state.outbox.push(reply); save(); publish(reply); publish(inventory());
  }
  try { await client.ready; } catch (e) { client.close(); rmdirSync(lock); throw e; }
  initialized = true;
  for (const m of state.outbox) publish(m);
  publish(inventory());
  const heartbeat = setInterval(() => {
    if (owned && (owned.exited || acp?.healthy === false) && state.phase === 'running') { state.phase = 'quarantined'; save(); }
    publish(inventory());
  },2000);
  return { agent, async close() {
    if (closing) return; closing = true;
    client.close(); clearInterval(heartbeat); acp?.cancel(); await queue;
    if (owned) {
      state.phase = 'transitioning'; save();
      try { await stopOwned(); state.phase = 'stopped'; state.actual = null; }
      catch (e) { state.phase = 'quarantined'; save(); throw e; }
    }
    save(); client.close();
    if (state.phase === 'stopped') rmdirSync(lock); // Failed teardown retains recovery fence.
  } };
}
