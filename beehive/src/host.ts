import { accessSync, constants, existsSync, mkdirSync, rmdirSync, realpathSync, statSync, readFileSync } from 'node:fs';
import { join, isAbsolute } from 'node:path';
import { createHash } from 'node:crypto';
import { hash, selection, validateAssignment, extendsAssignment, type Assignment, type Grant } from './handoff.ts';
import { installationSlots } from './slots.ts';
import { validateGenesis, type Genesis } from './assignment.ts';
import { connect, validateRelayURL } from './client.ts';
import { digest, fields, message, object, publicKey, text, type Message } from './protocol.ts';
import { readPrivate, writePrivate } from './storage.ts';
import { ConversationSession } from './broker.ts';
import { spawnOwned, type OwnedProcess } from './owned.ts';
import { AgentSession, prepareAgent, type AgentLaunch, type Catalog, type Evidence } from './acp.ts';
import { prepareConversation, conversationSummary, type ConversationSetup } from './conversation.ts';

export type Setup = { host: string; ownerSecret: string; agentSecret: string; runner: string; args: string[]; workspace: string; mode: 'fixture' | 'buzz-agent-databricks-v2'; databricksHost?: string; serviceHome?: string; configDirectory?: string; conversation?: ConversationSetup };
type Selection = { model: string; workspace: string; profile: string };
type State = { assignment: Assignment; move?: { request: Message; prepare: Message }; preparations?: Record<string, { request: Message; token: string; reply: Message }>; incoming?: Record<string, Message>; binding: { host: string; owner: string; agent: string }; revision: number; phase: 'stopped' | 'transitioning' | 'running' | 'quarantined'; selected: Selection; actual: null | { selection: Selection; executableHash: string; run: string; evidence?: Evidence }; operations: Record<string, { fingerprint: string; reply: Message }>; outbox: Message[] };
export function validateSetup(value: unknown): Setup {
  const s = object(value);
  for (const key of ['host','ownerSecret','agentSecret','runner','workspace']) text(s[key]);
  publicKey(String(s.ownerSecret)); publicKey(String(s.agentSecret));
  if (!isAbsolute(String(s.runner)) || !isAbsolute(String(s.workspace)) || !Array.isArray(s.args) || s.args.some(a => typeof a !== 'string') || !['fixture','buzz-agent-databricks-v2'].includes(String(s.mode))) throw Error('Invalid local setup');
  return s as Setup;
}
/** Provision once locally. A partial setup is inert; host startup never invents authority. */
export function provision(directory: string, value: Setup, root: Genesis): void {
  const setup = validateSetup(value); const genesis = validateGenesis(root);
  if (genesis.owner !== publicKey(setup.ownerSecret) || genesis.agent !== publicKey(setup.agentSecret)) throw Error('Genesis ownership mismatch');
  if (existsSync(join(directory, 'setup.json')) || existsSync(join(directory, 'journal.json'))) throw Error('Existing installation; cannot reset authority');
  writePrivate(join(directory, 'setup.json'), setup, true);
  writePrivate(join(directory, 'journal.json'), initialState(setup, genesis), true);
}
/** Initial stopped authority shared by legacy provisioning and slot enrollment. */
export function initialState(setup: Setup, genesis: Genesis): State {
  return {
    assignment: { genesis, assignedHost: genesis.initialHost },
    binding: { host: setup.host, owner: publicKey(setup.ownerSecret), agent: publicKey(setup.agentSecret) }, revision: 0, phase: 'stopped', selected: { model: setup.mode === 'fixture' ? 'fixture-model' : 'databricks-claude-haiku-4-5', workspace: setup.workspace, profile: 'default' }, actual: null, operations: {}, outbox: [],
  };
}
/** Explicit offline legacy enrollment; never repairs missing journals or lost keys. */
export function migrateAssignment(directory: string, genesis: Genesis): void {
  const lock = join(directory, 'host.lock'); mkdirSync(lock, { mode: 0o700 });
  try {
    const setup = validateSetup(readPrivate(join(directory, 'setup.json')));
    const state = readPrivate(join(directory, 'journal.json')) as State;
    validateGenesis(genesis);
    if (state.assignment) throw Error('Assignment already pinned; cannot replace');
    if (state.phase !== 'stopped' || state.actual !== null || state.binding.host !== setup.host || state.binding.agent !== publicKey(setup.agentSecret) || state.binding.owner !== publicKey(setup.ownerSecret) || genesis.agent !== state.binding.agent || genesis.owner !== state.binding.owner || genesis.initialHost !== setup.host) throw Error('Legacy migration requires matching stopped ownership');
    state.assignment = { genesis, assignedHost: genesis.initialHost };
    writePrivate(join(directory, 'journal.json'), state);
  } finally { rmdirSync(lock); }
}
/** Hosts consult durable assignment, never key presence or relay inventory, for authority. */
function slot(setup: Setup, path: string, currentSetup: () => Setup, publish: (m: Message) => void) {
  const agent = publicKey(setup.agentSecret);
  if (!existsSync(path)) throw Error('Authority journal missing; restore/reconcile locally, never re-enroll from key possession');
  let state = readPrivate(path) as State;
  if (state.binding.host !== setup.host || state.binding.owner !== publicKey(setup.ownerSecret) || state.binding.agent !== agent) throw Error('Saved ownership/assignment mismatch');
  if (!state.assignment) throw Error('Legacy assignment requires explicit local migrate-assignment while stopped');
  const genesis = validateGenesis(state.assignment.genesis);
  if (genesis.owner !== state.binding.owner || genesis.agent !== agent) throw Error('Saved assignment mismatch');
  validateAssignment(state.assignment);
  if (state.phase !== 'stopped') state.phase = 'quarantined';
  let persistenceFailed = false;
  const save = () => { try { writePrivate(path,state); } catch (e) { persistenceFailed = true; throw e; } };
  // Preparation is tied to this process lifetime. Never launch from a recovered token.
  const livePreparations = new Set<string>();
  let preparationStage = 'not-prepared';
  save();
  let restartProbe: AgentSession | undefined;
  let owned: OwnedProcess | ConversationSession | undefined;
  let acp: AgentSession | ConversationSession | undefined;
  let catalog: Catalog | { state: 'not-probed' | 'failed'; authentication: 'unverified' } = { state: 'not-probed', authentication: 'unverified' };
  function inventory() {
    return message('inventory',setup.host,agent,state.revision, {
      move: state.move ? 'destination preflight pending; source still assigned' : undefined, assignmentChain: state.assignment.chain ?? [], assignedHost: state.assignment.assignedHost, genesis: state.assignment.genesis, executionAuthority: state.assignment.assignedHost === setup.host, phase: state.phase, selectedNext: state.selected, actualRun: state.actual,
      ...(setup.conversation ? { conversation: conversationSummary(setup.conversation) } : {}),
      movePreflight: preparationStage, catalog, setup: setup.mode, models: [setup.mode === 'fixture' ? 'fixture-model' : 'databricks-claude-haiku-4-5'],
      workspaces: [setup.workspace], profiles: ['default'], readiness: setup.conversation ? (state.actual?.evidence ? 'Conversation session model acknowledged and response completed; relay delivery unverified' : 'Waiting for an admitted conversation and local provider sign-in; no synthetic prompt') : setup.mode === 'fixture' ? 'fixture-only' : (state.phase === 'running' && state.actual?.evidence ? 'ACP model acknowledged + same-session response (not provider attestation or Buzz relay agent)' : 'unverified: Start runs an ACP greeting probe; sign in locally as host service user if required'), observedAt: Date.now(),
    });
  }
  let closing = false;
  let queue = Promise.resolve();
  // Exact-fenced Stop retractions received ahead of their serialized handling, keyed
  // by that Stop's operation ID. One invariant covers every not-yet-committed Start
  // admission at the retracted revision, independent of callback/microtask timing:
  // a still-queued admission observes the retraction before any effect, and an
  // executing admission is interrupted below and re-checked before the running
  // commit. FIFO queue order confines each retraction to Starts received before its
  // Stop; that Stop's own serialized processing resolves it, so unseen future
  // Starts and already-committed runs are never cancelled by it.
  const queuedOperations = new Set<string>();
  const retracted = new Map<string, number>();
  const retracting = (revision: number) => { for (const target of retracted.values()) if (target === revision) return true; return false; };
  function receive(m: Message) {
    // Cancellation is signalled outside the serialized mutation queue. Admission is
    // still exact-authority/revision and never bypasses durable receipt processing.
    const operation = m.host === setup.host && ['save','start','restart','stop','move'].includes(m.type);
    const first = operation && !queuedOperations.has(m.id);
    if (operation) queuedOperations.add(m.id);
    // Reserve receive order too: a conflicting queued ID is not a Stop authority.
    if (first && !closing && m.type === 'stop' && m.host === setup.host && m.agent === agent && state.assignment.assignedHost === setup.host && m.revision === state.revision && !Object.hasOwn(state.operations, m.id) && Object.keys(m.body).length === 0) {
      retracted.set(m.id,m.revision);
      restartProbe?.cancel();
      // Only an executing admission has a session to interrupt; queued admissions
      // and the pre-commit recheck consume the retraction inside handle().
      if (state.phase === 'transitioning' && state.actual) acp?.cancel();
    }
    if (first && state.move && m.type === 'save' && m.revision === state.revision && state.assignment.assignedHost === setup.host && !Object.hasOwn(state.operations, m.id)) {
      try { const v = selection(m.body); if (v.model === state.selected.model && v.workspace === setup.workspace && v.profile === 'default') retracted.set(m.id, m.revision); } catch { /* Invalid Save cannot cancel. */ }
    }
    queue = queue.then(() => handle(m)).catch(() => { state.phase = 'quarantined'; save(); });
  }
  function replay() {
    for (const m of state.outbox) publish(m);
    if (state.move) publish(state.move.prepare);
    publish(inventory());
  }
  async function stopOwned() {
    const hadProbe = !!restartProbe;
    if (restartProbe) { await restartProbe.owned.stop(); restartProbe = undefined; }
    if (!owned && hadProbe) return;
    if (!owned) throw Error('Unknown ownership; quarantined');
    await owned.stop(); owned = undefined; acp = undefined;
  }
  function prepareLocal(selected: Selection) {
    selection(selected);
    if (selected.model !== (setup.mode === 'fixture' ? 'fixture-model' : 'databricks-claude-haiku-4-5') || selected.workspace !== setup.workspace || selected.profile !== 'default') throw Error('Unsupported destination selection');
    try {
      if (hash(currentSetup()) !== hash(setup)) throw Error('changed');
    } catch { throw Error('Local agent key/setup missing or changed; repair locally without resetting assignment'); }
    accessSync(setup.runner, constants.X_OK);
    if (realpathSync(setup.workspace) !== setup.workspace || !statSync(setup.workspace).isDirectory()) throw Error('Workspace changed');
    const launch: AgentLaunch | undefined = setup.mode === 'fixture' ? undefined : { executable: setup.runner, args: setup.args, workspace: selected.workspace, home: text(setup.serviceHome), configDirectory: text(setup.configDirectory), databricksHost: text(setup.databricksHost), model: selected.model };
    const prepared = launch ? prepareAgent(launch) : undefined;
    const conversation = setup.conversation && launch ? prepareConversation(setup.conversation, launch, setup.agentSecret, state.binding.owner) : undefined;
    // Local hashes only; no setup or secret values leave the host. Include existing
    // script inputs and directory identity so replacement invalidates preparation.
    const scripts = setup.args.filter(a => isAbsolute(a) && existsSync(a)).map(a => createHash('sha256').update(readFileSync(a)).digest('hex'));
    const workspace = statSync(setup.workspace);
    return { launch, token: hash({ setup, selected, prepared, conversation, scripts, executable: createHash('sha256').update(readFileSync(setup.runner)).digest('hex'), workspace: [workspace.dev, workspace.ino] }) };
  }
  /** Identity-free executable contract check, never conversation admission evidence. */
  async function inspectExecutable(executable: string, args: string[], required: string[]) {
    state.phase = 'transitioning'; save();
    owned = spawnOwned(executable, args, setup.workspace, { PATH: '/usr/bin:/bin' });
    const process = owned;
    try {
      await new Promise<void>((resolve, reject) => {
        let output = '', bytes = 0;
        const timer = setTimeout(() => reject(Error('Installed executable inspection timed out')), 5000);
        const finish = (error?: Error) => { clearTimeout(timer); error ? reject(error) : resolve(); };
        process.child.stdout.on('data', (chunk: Buffer) => {
          bytes += chunk.length;
          if (bytes > 128 * 1024) finish(Error('Installed executable inspection output limit'));
          else output += chunk.toString('utf8');
        });
        process.child.stderr.on('data', (chunk: Buffer) => {
          bytes += chunk.length;
          if (bytes > 128 * 1024) finish(Error('Installed executable inspection output limit'));
        });
        process.child.on('message', (m: any) => {
          if (m.type === 'runner-exit') finish(m.code === 0 && required.every(flag => output.includes(flag)) ? undefined : Error('Incompatible installed executable contract'));
        });
        void process.ready.catch(() => finish(Error('Installed executable unavailable')));
      });
    } finally {
      try { await stopOwned(); state.phase = 'stopped'; save(); }
      catch (error) { state.phase = 'quarantined'; save(); throw error; }
    }
  }
  function finishMove(result: string) {
    const pending = state.move;
    if (!pending) return;
    const request = pending.request, fingerprint = hash(request);
    const reply = message('receipt', setup.host, agent, state.revision, { operation: request.id, fingerprint, result, assignedHost: state.assignment.assignedHost, transfer: result === 'accepted' ? 'source consumed; grant queued; destination launch observed separately' : 'no grant' });
    state.operations[request.id] = { fingerprint, reply }; state.outbox.push(reply);
    delete state.move; save(); publish(reply); publish(inventory());
  }
  async function exchange(m: Message) {
    try {
      if (m.type === 'prepare') {
        fields(m.body, ['assignment','operation']);
        const assignment = m.body.assignment as Assignment; validateAssignment(assignment);
        const op = m.body.operation as Message;
        // Validate the proposed grant shape through the same chain validator.
        const g: Grant = { root: hash(assignment.genesis), predecessor: hash(assignment.chain?.at(-1) ?? assignment.genesis), source: op.host, target: setup.host, agent, operation: op, prepared: 'pending', selection: selection(op.body.selection), targetRevision: m.revision, sourceRun: null };
        validateAssignment({ ...assignment, assignedHost: setup.host, chain: [...(assignment.chain ?? []), g] });
        if (hash(assignment.genesis) !== hash(genesis) || !((state.assignment.chain ?? []).every((g, i) => hash(g) === hash(assignment.chain?.[i]))) || state.phase !== 'stopped' || state.assignment.assignedHost === setup.host || m.revision !== state.revision) throw Error('Destination authority/revision conflict');
        const prior = state.preparations?.[op.id];
        if (prior) {
          if (hash(prior.request) !== hash(m)) throw Error('Preparation ID conflict');
          // Grant-acceptance evidence is immutable, even after restart. Replayed
          // prepare cannot replace the token already consumed by the source.
          if (!livePreparations.has(op.id)) throw Error('Preparation lifetime ended; use a new Move operation');
          if (prepareLocal(g.selection).token !== prior.token) throw Error('Prepared inputs changed');
          publish(prior.reply); return;
        }
        if (!prior && Object.keys(state.preparations ?? {}).length >= 1000) throw Error('Preparation journal full; local reconciliation needed');
        preparationStage = 'checking-local-prerequisites; not conversation readiness'; publish(inventory());
        const prepared = prepareLocal(g.selection);
        // Independent, identity-free prerequisite probe. It receives neither agent
        // signer nor conversation relay/tool configuration. A successful probe is
        // NOT readiness for the future conversation, which must be verified afresh.
        if (setup.conversation) {
          await inspectExecutable(setup.conversation.executable, ['--help'], ['--agent-command', '--agent-args', '--agent-owner', '--respond-to', '--model']);
          if (setup.conversation.replyTool) await inspectExecutable(setup.conversation.replyTool.executable, ['messages', 'send', '--help'], ['--channel', '--reply-to', '--mention']);
        }
        if (prepared.launch) {
          state.phase = 'transitioning'; save();
          const probe = new AgentSession(prepared.launch); acp = probe; owned = probe.owned;
          try { await probe.catalog(); await probe.verify(); }
          finally {
            try { await stopOwned(); state.phase = 'stopped'; save(); }
            catch (error) { state.phase = 'quarantined'; save(); throw error; }
          }
        }
        if (closing || prepareLocal(g.selection).token !== prepared.token) throw Error('Preparation changed or host closing');
        const token = prepared.token;
        const reply = message('prepared', op.host, agent, op.revision, { prepare: m, token });
        state.preparations ??= {}; state.preparations[op.id] = { request: m, token, reply }; livePreparations.add(op.id); preparationStage = 'prepared; future actual conversation still unverified'; save(); publish(reply); publish(inventory());
      } else if (m.type === 'prepared') {
        const pending = state.move;
        if (!pending || hash(m.body.prepare) !== hash(pending.prepare) || m.revision !== pending.request.revision || state.revision !== m.revision || state.assignment.assignedHost !== setup.host) return;
        if (m.body.error) { finishMove(String(m.body.error)); return; }
        fields(m.body, ['prepare','token']); const token = text(m.body.token);
        if (state.phase !== 'stopped') {
          if (state.phase !== 'running' || !owned) throw Error('Source ownership unknown; no grant');
          state.phase = 'transitioning'; save(); await stopOwned();
        }
        // Stop/Save admitted during the await retract exactly this source revision.
        state.phase = 'stopped'; const sourceRun = state.actual?.run ?? null; state.actual = null;
        if (retracting(m.revision) || closing) { finishMove('Move cancelled after source Stop; no grant'); return; }
        const op = pending.request;
        const grant: Grant = { root: hash(genesis), predecessor: hash(state.assignment.chain?.at(-1) ?? genesis), source: setup.host, target: text(op.body.target), agent, operation: op, prepared: token, selection: selection(op.body.selection), targetRevision: Number(op.body.targetRevision), sourceRun };
        const assignment: Assignment = { genesis, assignedHost: grant.target, chain: [...(state.assignment.chain ?? []), grant] }; validateAssignment(assignment);
        const delivery = message('grant', grant.target, agent, grant.targetRevision, { assignment });
        // Irreversible authority consumption + exact grant outbox precede publication.
        state.assignment = assignment; state.revision++; state.outbox.push(delivery);
        finishMove('accepted'); publish(delivery);
      } else {
        fields(m.body, ['assignment']); const next = m.body.assignment as Assignment;
        validateAssignment(next); const g = next.chain?.at(-1);
        if (!g || g.target !== setup.host || g.agent !== agent || m.revision !== g.targetRevision) return;
        const prior = state.incoming?.[hash(g)];
        if (prior) { publish(prior); return; }
        if (!extendsAssignment(state.assignment, next) || state.assignment.assignedHost === setup.host || state.phase !== 'stopped') return;
        const prep = state.preparations?.[g.operation.id];
        if (!prep || hash(prep.request.body.operation) !== hash(g.operation) || prep.token !== g.prepared || state.revision !== g.targetRevision) return;
        if (hash(prep.request.body.assignment) !== hash({ genesis: next.genesis, assignedHost: g.source, ...(next.chain!.length > 1 ? { chain: next.chain!.slice(0,-1) } : {}) })) {
          // Genesis-only journals may explicitly retain an empty chain.
          const before = prep.request.body.assignment as Assignment;
          if (hash(before.genesis) !== hash(next.genesis) || before.assignedHost !== g.source || hash(before.chain ?? []) !== hash(next.chain!.slice(0,-1))) return;
        }
        // Accept before validating launch again: a stale/missing prerequisite can
        // fail launch but must never return authority to the consumed source.
        state.assignment = next; state.selected = g.selection;
        const receipt = message('receipt', setup.host, agent, state.revision, { operation: m.id, fingerprint: hash(m), result: 'destination assigned; launch pending' });
        state.incoming ??= {}; state.incoming[hash(g)] = receipt; state.outbox.push(receipt); save(); publish(receipt);
        let outcome: unknown;
        try {
          if (!livePreparations.has(g.operation.id) || prepareLocal(g.selection).token !== g.prepared) throw Error('Destination preparation invalidated; assigned here, stopped; repair local setup/key/auth then explicit Start');
          const start = message('start', setup.host, agent, state.revision); start.id = g.operation.id;
          await handle(start);
          outcome = state.operations[start.id]?.reply.body.result;
        } catch (e) { outcome = e instanceof Error ? e.message : 'Destination launch failed'; }
        livePreparations.delete(g.operation.id); preparationStage = 'consumed; actual launch outcome recorded separately';
        const completed = message('receipt', setup.host, agent, state.revision, { operation: m.id, fingerprint: hash(m), result: outcome, assignedHost: setup.host, transfer: 'source consumed; destination assigned' });
        state.incoming[hash(g)] = completed; state.outbox.push(completed); save(); publish(completed); publish(inventory());
      }
    } catch (e) {
      if (persistenceFailed) throw e;
      if (m.type === 'prepare') {
        preparationStage = 'failed; no source Stop authorized';
        if (state.phase === 'transitioning') { state.phase = 'quarantined'; save(); }
        publish(inventory());
        const op = m.body.operation as Message;
        if (op?.host) publish(message('prepared', op.host, agent, op.revision, { prepare: m, error: 'Destination preflight failed: locally provision matching key, allowed workspace and harness/auth setup; no source Stop' }));
      } else if (m.type === 'prepared') {
        if (state.phase === 'transitioning') state.phase = 'quarantined';
        finishMove(e instanceof Error ? e.message : 'Move failed; reconcile ownership');
      } // Invalid grants are inert; they cannot change local authority.
    }
  }
  async function handle(m: Message) {
    if (m.host === setup.host && ['save','start','restart','stop','move'].includes(m.type)) queuedOperations.delete(m.id);
    if (closing || persistenceFailed || m.host !== setup.host || !['inspect','save','start','restart','stop','move','prepare','prepared','grant'].includes(m.type)) return;
    if (['prepare','prepared','grant'].includes(m.type)) { await exchange(m); return; }
    // Serialized Stop processing durably resolves that Stop's own retraction.
    if (m.type === 'stop' || m.type === 'save') retracted.delete(m.id);
    if (m.type === 'inspect') { if (state.move) publish(state.move.prepare); for (const receipt of state.outbox) publish(receipt); publish(inventory()); return; }
    const fingerprint = digest(JSON.stringify(m)).toString('hex');
    const previous = Object.hasOwn(state.operations,m.id) ? state.operations[m.id] : undefined;
    if (previous) {
      // Move awaits a relay reply outside this queue: reservation is not terminal.
      if (state.move?.request.id === m.id && previous.fingerprint === fingerprint) {
        publish(state.move.prepare); publish(inventory()); return;
      }
      publish(previous.fingerprint === fingerprint ? previous.reply : message('receipt',setup.host,m.agent,state.revision,{ operation: m.id, fingerprint, result: 'operation-id-conflict' })); return;
    }
    let result = 'accepted';
    if (m.agent !== agent || state.assignment.assignedHost !== setup.host) result = 'not-authority';
    else if (state.move && ['start','restart','move'].includes(m.type)) result = 'Move reservation busy';
    else if (m.revision !== state.revision) result = 'revision-conflict';
    else if ((state.phase === 'quarantined' || state.phase === 'transitioning') && !(m.type === 'stop' && (restartProbe?.owned.child.connected || (owned instanceof ConversationSession ? owned.connected : owned?.child.connected)))) result = 'quarantined';
    else {
      // Reserve operation before any spawn/kill; interrupted admission never retries effects.
      const pending = message('receipt',setup.host,agent,state.revision,{ operation: m.id, fingerprint, result: 'interrupted-reconcile-locally' });
      state.operations[m.id] = { fingerprint, reply: pending }; save();
      try {
        if (state.move && m.type === 'stop') { fields(m.body, []); finishMove('Move cancelled before grant by stop'); }
        if (m.type === 'move') {
          fields(m.body, ['target','targetRevision','selection']);
          const target = text(m.body.target); selection(m.body.selection);
          if (target === setup.host || !Number.isSafeInteger(m.body.targetRevision) || Number(m.body.targetRevision) < 0 || (state.assignment.chain?.length ?? 0) >= 24) throw Error('Invalid destination or chain limit');
          const prepare = message('prepare', target, agent, Number(m.body.targetRevision), { assignment: state.assignment, operation: m });
          state.move = { request: m, prepare }; save(); publish(prepare); publish(inventory()); return;
        } else if (m.type === 'save') {
          fields(m.body,['model','workspace','profile']);
          const selected = { model: text(m.body.model), workspace: text(m.body.workspace), profile: text(m.body.profile) };
          if (selected.model !== (setup.mode === 'fixture' ? 'fixture-model' : 'databricks-claude-haiku-4-5') || selected.workspace !== setup.workspace || selected.profile !== 'default') throw Error('Unsupported selection');
          if (state.move) finishMove('Move cancelled before grant by save');
          state.selected = selected; result = 'saved; running configuration unchanged';
        } else if (m.type === 'start' || m.type === 'restart') {
          fields(m.body,[]);
          if (state.phase !== 'stopped' && !(m.type === 'restart' && state.phase === 'running')) throw Error('Already running');
          // A retracted admission never begins effects, including any spawn.
          if (retracting(m.revision)) throw Error('Start cancelled by concurrent Stop');
          const phaseBeforePreparation = state.phase;
          const selected = structuredClone(state.selected);
          const prepared = prepareLocal(selected);
          const { launch } = prepared;
          if (m.type === 'restart') {
            // Prerequisites are checked without replacing the old ownership handle
            // or passing the signer to a second identity-bearing runtime.
            if (launch) {
              const probe = new AgentSession(launch); restartProbe = probe;
              try { await probe.catalog(); await probe.verify(); }
              finally {
                try { await probe.owned.stop(); restartProbe = undefined; }
                catch (error) { state.phase = 'quarantined'; throw error; }
              }
            }
            if (closing || retracting(m.revision)) throw Error('Restart cancelled by Stop or host close');
            if (state.phase !== phaseBeforePreparation) throw Error('Restart ownership changed during preflight; reconcile before launch');
            if (prepareLocal(selected).token !== prepared.token) throw Error('Restart prepared inputs changed; existing run preserved');
            if (state.phase === 'running') {
              state.phase = 'transitioning'; save(); await stopOwned();
              state.phase = 'stopped'; state.actual = null; save();
            }
            if (closing || retracting(m.revision)) throw Error('Restart cancelled after Stop');
            if (prepareLocal(selected).token !== prepared.token) throw Error('Restart prepared inputs changed after Stop; assigned stopped');
          }
          state.actual = { selection: selected, executableHash: createHash('sha256').update(readFileSync(setup.runner)).digest('hex'), run: m.id };
          state.phase = 'transitioning'; save();
          if (setup.mode === 'fixture') {
            owned = spawnOwned(setup.runner, setup.args, state.actual.selection.workspace, { PATH: '/usr/bin:/bin', BEEHIVE_FIXTURE: '1' });
            owned.child.stdout.resume(); owned.child.stderr.resume();
            try { await owned.ready; } catch (e) { await stopOwned(); state.phase = 'stopped'; state.actual = null; throw e; }
          } else if (setup.conversation) {
            const session = new ConversationSession(setup.conversation, launch!, setup.agentSecret, state.binding.owner);
            acp = session; owned = session;
            try {
              state.actual.evidence = await session.verify();
              if (!session.healthy) throw Error('Conversation failed before running commit');
            } catch (e) {
              await stopOwned(); state.phase = 'stopped'; state.actual = null;
              if (retracting(m.revision)) throw Error('Start cancelled by concurrent Stop');
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
              if (retracting(m.revision)) throw Error('Start cancelled by concurrent Stop');
              throw e;
            }
          }
          // The retraction may also arrive while this admission executes: verified
          // teardown must precede the running commit, mirroring the queued path.
          if (retracting(m.revision)) {
            await stopOwned(); state.phase = 'stopped'; state.actual = null;
            throw Error('Start cancelled by concurrent Stop');
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
        if (persistenceFailed) throw error;
        if (state.phase === 'transitioning') state.phase = 'quarantined';
        result = error instanceof Error ? error.message : 'Operation failed';
      }
    }
    const reply = message('receipt',setup.host,m.agent,state.revision,{ operation: m.id, fingerprint, result });
    Object.defineProperty(state.operations,m.id,{ value: { fingerprint, reply }, enumerable: true, configurable: true, writable: true });
    state.outbox.push(reply); save(); publish(reply); publish(inventory());
  }
  return { agent, receive, replay, heartbeat() {
    if (owned && (owned.exited || acp?.healthy === false) && state.phase === 'running') { state.phase = 'quarantined'; save(); }
    publish(inventory());
  }, async close() {
    if (closing) return; closing = true;
    restartProbe?.cancel(); acp?.cancel(); await queue;
    if (restartProbe) { await restartProbe.owned.stop(); restartProbe = undefined; }
    if (owned) {
      state.phase = 'transitioning'; save();
      try { await stopOwned(); state.phase = 'stopped'; state.actual = null; }
      catch (e) { state.phase = 'quarantined'; save(); throw e; }
    }
    save();
    if (state.phase !== 'stopped') throw Error(`Agent ${agent}: unknown execution; installation remains locked`);
  } };
}

/** One installation owns every slot, lock and management transport. */
export async function host(directory: string, url: string) {
  validateRelayURL(url);
  const lock = join(directory, 'host.lock');
  mkdirSync(lock, { mode: 0o700 }); // Never infer ownership from a recovered PID.
  let client: ReturnType<typeof connect> | undefined;
  let initialized = false;
  const slots = new Map<string, ReturnType<typeof slot>>();
  const publish = (m: Message) => { try { client?.send(m); } catch { /* Slot outbox survives transport loss. */ } };
  try {
    const entries = installationSlots(directory);
    for (const entry of entries) {
      slots.set(publicKey(entry.setup.agentSecret), slot(entry.setup, entry.path, () => {
        const current = installationSlots(directory).find(e => publicKey(e.setup.agentSecret) === publicKey(entry.setup.agentSecret));
        if (!current) throw Error('Key removed');
        return current.setup;
      }, publish));
    }
    const setup = entries[0]!.setup;
    // Every slot is hydrated before dialing. Initial WS history may arrive in the
    // same event-loop turn as open, before the ready promise continuation.
    initialized = true;
    client = connect(url, setup.ownerSecret, m => {
      if (!initialized || m.host !== setup.host) return;
      const selected = slots.get(m.agent);
      if (selected) selected.receive(m);
      else if (m.type === 'inspect') { for (const s of slots.values()) s.replay(); }
      else if (['save','start','restart','stop','move'].includes(m.type)) publish(message('receipt',setup.host,m.agent,0,{ operation: m.id, fingerprint: digest(JSON.stringify(m)).toString('hex'), result: 'not-authority' }));
    }, () => { if (initialized) for (const s of slots.values()) s.replay(); });
    await client.ready;
  } catch (e) { client?.close(); rmdirSync(lock); throw e; }
  initialized = true;
  for (const s of slots.values()) s.replay();
  const heartbeat = setInterval(() => { for (const s of slots.values()) s.heartbeat(); }, 2000);
  let closing: Promise<void> | undefined;
  return { agent: [...slots.keys()][0]!, agents: [...slots.keys()], close() {
    return closing ??= (async () => {
      initialized = false; client?.close(); clearInterval(heartbeat);
      // allSettled starts every teardown and preserves each slot's durable truth.
      const results = await Promise.allSettled([...slots.values()].map(s => s.close()));
      const errors = results.filter((r): r is PromiseRejectedResult => r.status === 'rejected').map(r => r.reason);
      if (errors.length) throw new AggregateError(errors, 'Incomplete owned teardown; installation remains locked');
      rmdirSync(lock);
    })();
  } };
}
