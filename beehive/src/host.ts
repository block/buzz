import { spawn, type ChildProcess } from 'node:child_process';
import { accessSync, constants, existsSync, mkdirSync, rmdirSync, realpathSync, statSync, readFileSync } from 'node:fs';
import { join, isAbsolute } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { connect } from './client.ts';
import { digest, fields, message, object, publicKey, text, type Message } from './protocol.ts';
import { readPrivate, writePrivate } from './storage.ts';

export type Setup = { host: string; ownerSecret: string; agentSecret: string; runner: string; args: string[]; workspace: string; mode: 'fixture' | 'buzz-agent-databricks-v2'; databricksHost?: string };
type Selection = { model: string; workspace: string; profile: string };
type State = { binding: { host: string; owner: string; agent: string }; revision: number; phase: 'stopped' | 'transitioning' | 'running' | 'quarantined'; selected: Selection; actual: null | { selection: Selection; executableHash: string; run: string }; operations: Record<string, { fingerprint: string; reply: Message }>; outbox: Message[] };
export function validateSetup(value: unknown): Setup {
  const s = object(value);
  for (const key of ['host','ownerSecret','agentSecret','runner','workspace']) text(s[key]);
  publicKey(String(s.ownerSecret)); publicKey(String(s.agentSecret));
  if (!isAbsolute(String(s.runner)) || !isAbsolute(String(s.workspace)) || !Array.isArray(s.args) || s.args.some(a => typeof a !== 'string') || !['fixture','buzz-agent-databricks-v2'].includes(String(s.mode))) throw Error('Invalid local setup');
  return s as Setup;
}
/** Fixed-authority first slice. Keys never constitute a remote assignment grant. */
export async function host(directory: string, url: string) {
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
  save();
  let child: ChildProcess | undefined;
  function inventory() {
    return message('inventory',setup.host,agent,state.revision, {
      assignedHost: setup.host, phase: state.phase, selectedNext: state.selected, actualRun: state.actual,
      setup: setup.mode, models: [setup.mode === 'fixture' ? 'fixture-model' : 'databricks-claude-haiku-4-5'],
      workspaces: [setup.workspace], profiles: ['default'], readiness: setup.mode === 'fixture' ? 'fixture-only' : 'blocked: exact-model application evidence not implemented', observedAt: Date.now(),
    });
  }
  let closing = false;
  let queue = Promise.resolve();
  const client = connect(url,setup.ownerSecret,m => { queue = queue.then(() => handle(m)).catch(() => { state.phase = 'quarantined'; save(); }); });
  const publish = (m: Message) => { try { client.send(m); } catch { /* Durable outbox retained for reconnect/restart. */ } };
  async function stopOwned() {
    if (!child?.pid) throw Error('Unknown ownership; quarantined');
    const pid = child.pid;
    // Child handle from this host lifetime is required; never recover ownership from PID.
    const exited = child.exitCode !== null || child.signalCode !== null;
    if (!exited) process.kill(-pid,'SIGTERM');
    for (let i = 0; i < 100; i++) {
      try { process.kill(-pid,0); } catch (e) {
        if ((e as NodeJS.ErrnoException).code === 'ESRCH') { child = undefined; return; }
        throw e;
      }
      await delay(25);
    }
    throw Error('Owned process group did not exit; quarantined');
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
    else if (state.phase === 'quarantined' || state.phase === 'transitioning') result = 'quarantined';
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
          if (setup.mode !== 'fixture') throw Error('Exact-model application evidence unavailable');
          accessSync(setup.runner,constants.X_OK);
          if (realpathSync(setup.workspace) !== setup.workspace || !statSync(setup.workspace).isDirectory()) throw Error('Workspace changed');
          state.actual = { selection: structuredClone(state.selected), executableHash: digest(readFileSync(setup.runner).toString('base64')).toString('hex'), run: m.id };
          state.phase = 'transitioning'; save();
          child = spawn(setup.runner,setup.args,{ cwd: state.selected.workspace, detached: true, stdio: 'ignore', env: { PATH: '/usr/bin:/bin', BEEHIVE_FIXTURE: '1' } });
          await new Promise<void>((resolve,reject) => { child!.once('spawn',resolve); child!.once('error',reject); });
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
  await client.ready;
  for (const m of state.outbox) publish(m);
  publish(inventory());
  const heartbeat = setInterval(() => {
    if (child && (child.exitCode !== null || child.signalCode !== null) && state.phase === 'running') { state.phase = 'quarantined'; save(); }
    publish(inventory());
  },2000);
  return { agent, async close() {
    if (closing) return; closing = true;
    client.close(); clearInterval(heartbeat); await queue;
    if (state.phase === 'running') { state.phase = 'transitioning'; save(); try { await stopOwned(); state.phase = 'stopped'; state.actual = null; } catch { state.phase = 'quarantined'; } }
    save(); client.close(); rmdirSync(lock);
  } };
}
