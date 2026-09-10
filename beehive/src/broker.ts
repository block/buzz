import { setTimeout as delay } from 'node:timers/promises';
import { createServer, type Socket } from 'node:net';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash, randomBytes } from 'node:crypto';
import { spawnOwned, type OwnedProcess } from './owned.ts';
import { prepareAgent, type AgentLaunch, type Evidence } from './acp.ts';
import { prepareConversation, type ConversationSetup } from './conversation.ts';

type RPC = { jsonrpc: string; id?: string | number; method?: string; params?: any; result?: any; error?: unknown };
type Pending = { method: string; params: any };
const validID = (s: unknown): s is string => typeof s === 'string' && /^[a-zA-Z0-9_.:/-]{1,200}$/.test(s);

/** Run-local ACP transport. Authority comes only from the locally snapshotted plan. */
export class ConversationSession {
  private directory: string;
  private capability = randomBytes(32).toString('hex');
  private sockets = new Set<Socket>();
  private harnesses: OwnedProcess[] = [];
  private shimPids = new Set<number>();
  private runtime?: OwnedProcess;
  private cancellations: (() => void)[] = [];
  private failed = false;
  private stopping?: Promise<void>;
  private resolve!: (e: Evidence) => void;
  private reject!: (e: Error) => void;
  private evidence = new Promise<Evidence>((resolve, reject) => { this.resolve = resolve; this.reject = reject; });
  private server = createServer(socket => this.accept(socket));
  private prepared;
  private plan;
  private timeout: ReturnType<typeof setTimeout>;
  readonly ready: Promise<void>;
  constructor(setup: ConversationSetup, launch: AgentLaunch, secret: string, owner: string, timeoutMs = 30_000) {
    this.prepared = prepareAgent(launch);
    this.plan = prepareConversation(setup, launch, secret, owner);
    this.directory = mkdtempSync(join(tmpdir(), 'bh-'));
    // Rejection can precede verify() while the local listener is opening.
    void this.evidence.catch(() => {});
    this.timeout = setTimeout(() => this.fail('Conversation response timed out; check relay admission and local sign-in'), timeoutMs);
    this.server.on('error', () => this.fail('Private ACP broker unavailable'));
    this.ready = new Promise<void>((resolve, reject) => {
      this.server.once('error', reject);
      this.server.listen(join(this.directory, 'acp'), () => {
        if (this.failed) { reject(Error('Conversation Start cancelled')); return; }
        const shim = fileURLToPath(new URL('./acp-shim.ts', import.meta.url));
        const args = [shim, join(this.directory, 'acp'), this.capability];
        if (args.some(a => a.includes(','))) { reject(Error('Unsupported local shim path')); return; }
        this.runtime = spawnOwned(this.plan.executable, [], this.plan.workspace, {
          ...this.plan.env, BUZZ_ACP_AGENT_COMMAND: process.execPath, BUZZ_ACP_AGENT_ARGS: args.join(','),
        });
        this.runtime.child.stdout.resume(); this.runtime.child.stderr.resume();
        this.watch(this.runtime);
        this.runtime.ready.then(resolve, reject);
      });
    });
    void this.ready.catch(() => this.fail('Conversation runtime unavailable'));
  }
  get healthy() { return !this.failed; }
  get exited() { return this.failed || !!this.runtime?.exited; }
  get connected() { return !this.stopping || !this.failed; }
  private watch(owned: OwnedProcess) {
    owned.child.on('message', () => { if (owned.exited) this.fail('Conversation process exited'); });
    owned.child.on('exit', () => this.fail('Conversation supervisor exited'));
    owned.child.on('error', () => this.fail('Conversation process unavailable'));
    void owned.ready.catch(() => this.fail('Conversation harness unavailable'));
  }
  private fail(reason: string) {
    if (this.failed) return;
    this.failed = true; clearTimeout(this.timeout); this.reject(Error(reason));
    for (const socket of this.sockets) socket.destroy();
    // Keep ownership handles until the caller verifies complete teardown.
  }
  /** Interrupt Start without allowing late ACP responses to commit readiness. */
  cancel() {
    if (!this.failed) for (const cancel of this.cancellations) cancel();
    this.fail('Conversation Start cancelled');
  }
  /** Evidence is from the external runtime's actual prompt, never a synthetic probe. */
  async verify() {
    await this.ready;
    const evidence = await this.evidence;
    if (!this.healthy) throw Error('Conversation failed before evidence commit');
    return evidence;
  }
  /** Close escaping, childless shims via EOF and stop every retained live group. */
  stop(): Promise<void> {
    if (this.stopping) return this.stopping;
    this.cancel();
    this.stopping = (async () => {
      await this.ready.catch(() => {});
      await new Promise<void>(resolve => this.server.close(() => resolve()));
      const results = await Promise.allSettled([...this.harnesses, ...(this.runtime ? [this.runtime] : [])].map(p => p.stop()));
      if (results.some(r => r.status === 'rejected')) throw Error('Conversation ownership teardown failed; reconcile locally');
      // These numeric IDs are absence observations ONLY, never kill authority.
      // A shim is childless and exits on EOF even though upstream gave it a new PGID.
      for (let i = 0; i < 100 && this.shimPids.size; i++) {
        for (const pid of this.shimPids) {
          try { process.kill(pid, 0); }
          catch (error) { if ((error as NodeJS.ErrnoException).code === 'ESRCH') this.shimPids.delete(pid); }
        }
        if (this.shimPids.size) await delay(25);
      }
      if (this.shimPids.size) throw Error('ACP shim exit unconfirmed; reconcile locally');
      rmSync(this.directory, { recursive: true, force: true });
    })();
    return this.stopping;
  }
  private accept(socket: Socket) {
    if (this.failed || this.sockets.size >= 8) { socket.destroy(); return; }
    this.sockets.add(socket);
    let authenticated = false;
    let buffer = '';
    let harness: OwnedProcess | undefined;
    const timer = setTimeout(() => socket.destroy(), 2000);
    socket.on('error', () => socket.destroy());
    socket.on('close', () => {
      clearTimeout(timer); this.sockets.delete(socket);
      if (authenticated && !this.failed) this.fail('ACP shim disconnected');
    });
    socket.setEncoding('utf8');
    socket.on('data', (chunk: string) => {
      if (authenticated) return;
      buffer += chunk;
      if (buffer.length > 4096) { socket.destroy(); return; }
      const end = buffer.indexOf('\n');
      if (end < 0) return;
      let hello: { capability?: string; pid?: number };
      try { hello = JSON.parse(buffer.slice(0, end)); } catch { socket.destroy(); return; }
      if (!hello || hello.capability !== this.capability || !Number.isSafeInteger(hello.pid) || hello.pid! <= 1 || this.harnesses.length >= 8) { socket.destroy(); return; }
      this.shimPids.add(hello.pid!);
      authenticated = true; clearTimeout(timer);
      const { plan, env } = this.prepared;
      // No data supplied by a shim can alter argv, workspace, identity or env.
      harness = spawnOwned(plan.executable, plan.args, plan.workspace, {
        ...env, BUZZ_PRIVATE_KEY: this.plan.env.BUZZ_PRIVATE_KEY,
        BUZZ_RELAY_URL: this.plan.env.BUZZ_RELAY_URL,
        ...(this.plan.env.BUZZ_AUTH_TAG ? { BUZZ_AUTH_TAG: this.plan.env.BUZZ_AUTH_TAG } : {}),
      });
      this.harnesses.push(harness); this.watch(harness);
      this.proxy(socket, harness, buffer.slice(end + 1));
      buffer = '';
    });
  }
  private proxy(socket: Socket, harness: OwnedProcess, initial: string) {
    const pending = new Map<string | number, Pending>();
    const sessions = new Set<string>();
    const modelConfigs = new Set<string>(['model']);
    const prompts = new Map<string, { id: string | number; hash: ReturnType<typeof createHash>; text: boolean; cancelled: boolean }>();
    this.cancellations.push(() => {
      for (const sessionId of prompts.keys()) harness.child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'session/cancel', params: { sessionId } }) + '\n');
    });
    const injected = new Map<string, { original: RPC; session: string }>();
    let sequence = 0;
    const send = (target: NodeJS.WritableStream, msg: RPC) => {
      if (this.failed) return;
      if (!target.write(JSON.stringify(msg) + '\n')) this.fail('ACP transport backpressure limit');
    };
    const inspect = (msg: RPC, fromHarness: boolean) => {
      if (msg.jsonrpc !== '2.0' || (msg.id !== undefined &&
        !(typeof msg.id === 'string' && msg.id.length <= 200) &&
        !(typeof msg.id === 'number' && Number.isSafeInteger(msg.id)))) throw Error();
      if (msg.method !== undefined) {
        if (typeof msg.method !== 'string' || !msg.method.length || msg.result !== undefined || msg.error !== undefined) throw Error();
      } else if (msg.id === undefined || (Object.hasOwn(msg, 'result') === Object.hasOwn(msg, 'error'))) throw Error();
      if (!fromHarness && msg.method) {
        const p = msg.params;
        // Session transport does not grant a shim authority to provision tools,
        // workspaces or credentials. This slice has no locally approved MCP plan.
        if (['session/new', 'session/load', 'session/resume'].includes(msg.method)) {
          if (p?.cwd !== this.prepared.plan.workspace || !Array.isArray(p.mcpServers) || p.mcpServers.length) throw Error();
          sessions.delete(p.sessionId);
        }
        if (p && ['env', 'environment', 'executable', 'command'].some(key => Object.hasOwn(p, key))) throw Error();
        if (msg.id !== undefined) {
          if (pending.size >= 128 || pending.has(msg.id) || String(msg.id).startsWith('beehive-model-')) throw Error();
          pending.set(msg.id, { method: msg.method, params: p });
        }
        if (msg.method === 'session/set_model' && (!sessions.has(p?.sessionId) || p.modelId !== this.prepared.plan.model)) throw Error();
        if (msg.method === 'session/set_config_option' && modelConfigs.has(p?.configId) && p.value !== this.prepared.plan.model) throw Error();
        if (['session/set_model', 'session/set_config_option'].includes(msg.method)) sessions.delete(p?.sessionId);
        if (msg.method === 'session/prompt') {
          if (!sessions.has(p?.sessionId) || prompts.has(p.sessionId) || msg.id === undefined) throw Error();
          prompts.set(p.sessionId, { id: msg.id, hash: createHash('sha256'), text: false, cancelled: false });
        }
        if (msg.method === 'session/cancel') {
          const prompt = prompts.get(p?.sessionId); if (prompt) prompt.cancelled = true;
        }
      }
      if (fromHarness && !msg.method && msg.id !== undefined) {
        const internal = injected.get(String(msg.id));
        if (internal) {
          injected.delete(String(msg.id));
          if (msg.error !== undefined || msg.result?.sessionId !== internal.session || msg.result?.modelId !== this.prepared.plan.model) throw Error();
          sessions.add(internal.session); send(socket, internal.original); return;
        }
        const request = pending.get(msg.id);
        if (!request) throw Error();
        pending.delete(msg.id);
        if (['session/new', 'session/load', 'session/resume', 'session/set_config_option'].includes(request.method) && msg.error === undefined) {
          const session = msg.result?.sessionId ?? request.params?.sessionId;
          if (!validID(session) || sessions.size >= 128 || injected.size >= 128) throw Error();
          for (const option of msg.result?.configOptions ?? []) {
            if (option.category === 'model' && typeof option.id === 'string') modelConfigs.add(option.id);
            if (option.category === 'model' && typeof option.configId === 'string') modelConfigs.add(option.configId);
          }
          if (modelConfigs.size > 128) throw Error();
          sessions.delete(session);
          const id = `beehive-model-${++sequence}`;
          injected.set(id, { original: msg, session });
          send(harness.child.stdin, { jsonrpc: '2.0', id, method: 'session/set_model', params: { sessionId: session, modelId: this.prepared.plan.model } }); return;
        }
        if (request.method === 'session/set_model') {
          if (msg.error !== undefined || msg.result?.sessionId !== request.params.sessionId || msg.result?.modelId !== this.prepared.plan.model) throw Error();
          sessions.add(request.params.sessionId);
        }
        if (request.method === 'session/prompt') {
          const session = request.params.sessionId; const prompt = prompts.get(session); prompts.delete(session);
          if (msg.error === undefined && msg.result?.stopReason === 'end_turn' && prompt?.text && !prompt.cancelled) {
            const evidence: Evidence = { session, model: this.prepared.plan.model, executableHash: this.prepared.executableHash, responseHash: prompt.hash.digest('hex'), stopReason: 'end_turn', source: 'external-buzz-conversation', agentPublicKey: this.plan.agentPublicKey, runtimeExecutableHash: this.plan.executableHash };
            // Defer commit until all lines in this batch have been validated (D3).
            queueMicrotask(() => { if (this.healthy) { clearTimeout(this.timeout); this.resolve(evidence); } });
          }
        }
      }
      if (fromHarness && msg.method === 'session/update') {
        const p = msg.params; const u = p?.update;
        if (u?.sessionUpdate === 'current_model_update' && u.currentModelId !== this.prepared.plan.model) throw Error();
        if (u?.sessionUpdate === 'config_option_update' && Array.isArray(u.configOptions)) {
          for (const option of u.configOptions) if (option.category === 'model' && option.currentValue !== this.prepared.plan.model) throw Error();
        }
        const prompt = prompts.get(p?.sessionId);
        if (prompt && u?.sessionUpdate === 'agent_message_chunk' && u.content?.type === 'text' && typeof u.content.text === 'string') {
          prompt.hash.update(u.content.text); prompt.text ||= !!u.content.text.trim();
        }
      }
      send(fromHarness ? socket : harness.child.stdin, msg);
    };
    const reader = (fromHarness: boolean) => {
      let buffer = '';
      return (chunk: string) => {
        if (this.failed) return;
        buffer += chunk;
        if (Buffer.byteLength(buffer) > 1024 * 1024) { this.fail('ACP frame limit exceeded'); return; }
        try {
          let end: number;
          while ((end = buffer.indexOf('\n')) >= 0) {
            const line = buffer.slice(0, end); buffer = buffer.slice(end + 1);
            if (line.trim()) { const msg = JSON.parse(line); if (!msg || typeof msg !== 'object' || Array.isArray(msg)) throw Error(); inspect(msg, fromHarness); }
          }
        } catch { this.fail('Conversation ACP protocol/model verification failed (diagnostics withheld)'); }
      };
    };
    const inbound = reader(false);
    socket.on('data', inbound);
    harness.child.stdout.setEncoding('utf8'); harness.child.stdout.on('data', reader(true));
    harness.child.stdin.on('error', () => this.fail('ACP input failed'));
    harness.child.stderr.resume();
    if (initial) inbound(initial);
  }
}
