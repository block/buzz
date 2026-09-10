import { validateCustom, type CustomAcp } from './custom-acp.ts';
import { CODEX_ADAPTER, codexEnvironment, codexModels, type CodexSetup } from './codex.ts';
import { CLAUDE_ADAPTER, claudeEnvironment, claudeModels, type ClaudeSetup } from './claude.ts';
import { type ChildProcessWithoutNullStreams } from 'node:child_process';
import { accessSync, constants, readFileSync, realpathSync, statSync } from 'node:fs';
import { isAbsolute } from 'node:path';
import { createHash } from 'node:crypto';
import { spawnOwned, type OwnedProcess } from './owned.ts';

/** Host-prepared launch: executable/provider binding stays local; instructions are a validated public snapshot. Never accept remote env/argv. */
export type AgentLaunch = Readonly<{ custom?: CustomAcp; executable: string; args: readonly string[]; workspace: string; home: string; configDirectory: string; databricksHost: string; harness?: 'goose' | 'claude' | 'codex'; codex?: CodexSetup; claude?: ClaudeSetup; provider?: string; model: string; instructions?: string }>;
/** ACP catalogs can be fallback data; even a nonempty result is NOT auth evidence. */
export type Catalog = { state: 'reported' | 'empty' | 'filtered'; models: string[]; authentication: 'unverified' };
/** Same child/session acknowledgement plus completed text response, not provider attestation. */
export type Evidence = { session: string; model: string; responseHash: string; stopReason: 'end_turn'; source: 'external-acp-session' | 'external-buzz-conversation'; executableHash: string; agentPublicKey?: string; runtimeExecutableHash?: string };
const hash = (s: string | Buffer) => createHash('sha256').update(s).digest('hex');
const record = (v: unknown): Record<string, unknown> => {
  if (!v || typeof v !== 'object' || Array.isArray(v)) throw Error('Invalid ACP response');
  return v as Record<string, unknown>;
};
const identifier = (v: unknown): string => {
  if (typeof v !== 'string' || !/^[a-zA-Z0-9_.:/-]{1,200}$/.test(v)) throw Error('Invalid ACP identifier');
  return v;
};
/** Validate and snapshot local launch settings. No ambient provider credentials inherited. */
export function prepareAgent(input: AgentLaunch) {
  const plan = Object.freeze({ ...input, args: Object.freeze([...input.args]) });
  for (const p of [plan.executable, plan.workspace, plan.home, plan.configDirectory]) {
    if (!isAbsolute(p) || realpathSync(p) !== p) throw Error('Harness setup requires canonical absolute paths');
  }
  accessSync(plan.executable, constants.X_OK);
  for (const p of [plan.workspace, plan.home, plan.configDirectory]) if (!statSync(p).isDirectory()) throw Error('Harness setup directory unavailable');
  if (plan.harness === 'codex') {
    identifier(plan.model);
    if (plan.args.length || !plan.codex || !plan.codex.models.includes(plan.model) || plan.home === plan.configDirectory) throw Error('Codex requires zero-argument adapter, approved model and distinct HOME/CODEX_HOME');
    return Object.freeze({ plan, executableHash: hash(readFileSync(plan.executable)), env: Object.freeze(codexEnvironment(plan.codex, plan.home, plan.configDirectory, plan.model)), cliExecutableHash: hash(readFileSync(plan.codex.cli)) });
  }
  if (plan.harness === 'claude') {
    identifier(plan.model);
    if (plan.args.length || !plan.claude) throw Error('Claude requires a zero-argument ACP adapter and local CLI/key binding');
    return Object.freeze({ plan, executableHash: hash(readFileSync(plan.executable)), env: Object.freeze(claudeEnvironment(plan.claude, plan.home, plan.model)), cliExecutableHash: hash(readFileSync(plan.claude.cli)) });
  }
  if (plan.custom) {
    const d = validateCustom(plan.custom);
    if (plan.harness !== 'goose' || d.contract !== 'goose-native' || d.executable !== plan.executable || JSON.stringify(d.args) !== JSON.stringify(plan.args)) throw Error('Unsupported custom launch contract');
  }
  if (plan.harness === 'goose') {
    identifier(plan.provider);
    identifier(plan.model);
    return Object.freeze({ plan, executableHash: hash(readFileSync(plan.executable)), env: Object.freeze<Record<string, string>>({ ...plan.custom?.env, PATH: '/usr/bin:/bin', HOME: plan.home, GOOSE_PROVIDER: plan.provider!, GOOSE_MODEL: plan.model, GOOSE_MODE: 'auto' }) });
  }
  const url = new URL(plan.databricksHost);
  if (url.protocol !== 'https:' || url.username || url.password || url.search || url.hash || url.pathname !== '/') throw Error('Databricks workspace must be an HTTPS origin');
  identifier(plan.model);
  return Object.freeze({ plan, executableHash: hash(readFileSync(plan.executable)), env: Object.freeze<Record<string, string>>({ PATH: '/usr/bin:/bin', HOME: plan.home, BUZZ_AGENT_CONFIG_DIR: plan.configDirectory, BUZZ_AGENT_PROVIDER: 'databricks_v2', DATABRICKS_HOST: url.origin, BUZZ_AGENT_MODEL: plan.model, ...(plan.instructions === undefined ? {} : { BUZZ_AGENT_SYSTEM_PROMPT: plan.instructions }) }) });
}

/** Spawn-fixed Goose model evidence from native ACP configOptions, not set_model.
 * Pinned Buzz 051c3a2 catalog disables Goose unstable model switching. No fallback.
 */
export function gooseModels(value: unknown, model: string): string[] {
  const options = record(value).configOptions;
  if (!Array.isArray(options) || options.length > 128) throw Error('Missing Goose native model evidence');
  const models = options.filter(o => record(o).category === 'model');
  if (models.length !== 1) throw Error('Ambiguous Goose model evidence');
  const option = record(models[0]);
  if (option.currentValue !== model || !Array.isArray(option.options) || option.options.length > 1000) throw Error('Goose exact-model mismatch');
  const ids = option.options.map(o => identifier(record(o).value));
  if (!ids.includes(model)) throw Error('Goose model not advertised');
  return ids;
}

/** Bounded newline JSON-RPC boundary to an external Buzz Agent ACP executable. */
export class AgentSession {
  readonly child: ChildProcessWithoutNullStreams;
  readonly owned: OwnedProcess;
  readonly prepared: ReturnType<typeof prepareAgent>;
  private sequence = 0;
  private pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void; timer: ReturnType<typeof setTimeout> }>();
  private buffer = '';
  private bytes = 0;
  private failed = false;
  private session = '';
  private codexModelVerified = false;
  private response = '';
  private prompting = false;
  private timeoutMs: number;
  constructor(input: AgentLaunch, timeoutMs = 30_000) {
    this.timeoutMs = timeoutMs;
    this.prepared = prepareAgent(input);
    const { plan, env } = this.prepared;
    this.owned = spawnOwned(plan.executable, plan.args, plan.workspace, env);
    this.child = this.owned.child;
    void this.owned.ready.catch(() => this.fail('Harness executable unavailable'));
    this.child.on('message', () => { if (this.owned.exited) this.fail('Harness exited'); });
    this.child.stdin.on('error', () => this.fail('ACP input failed'));
    this.child.on('error', () => this.fail('Harness executable unavailable'));
    this.child.on('exit', () => this.fail('Harness exited'));
    // Never retain or relay raw external diagnostics (may contain credentials).
    this.child.stderr.on('data', (chunk: Buffer) => this.account(chunk.length));
    this.child.stdout.setEncoding('utf8');
    this.child.stdout.on('data', (chunk: string) => {
      if (!this.account(Buffer.byteLength(chunk))) return;
      this.buffer += chunk;
      try {
        let end: number;
        while ((end = this.buffer.indexOf('\n')) >= 0) {
          const line = this.buffer.slice(0, end); this.buffer = this.buffer.slice(end + 1);
          if (line.trim()) this.receive(record(JSON.parse(line)));
        }
      } catch { this.fail('Malformed ACP protocol'); }
    });
  }
  private account(n: number) {
    this.bytes += n;
    if (this.bytes > 1024 * 1024) this.fail('ACP output limit exceeded');
    return !this.failed;
  }
  private fail(reason: string) {
    if (this.failed) return;
    this.failed = true;
    // Caller retains the live group anchor for verified Stop, including protocol failures.
    for (const p of this.pending.values()) { clearTimeout(p.timer); p.reject(Error(reason)); }
    this.pending.clear();
  }
  /** Interrupt outstanding startup work; late completions cannot commit readiness. */
  cancel() {
    if (this.failed) return;
    if (this.session && this.prompting) this.child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'session/cancel', params: { sessionId: this.session } }) + '\n');
    this.fail('ACP Start cancelled');
  }
  /** Protocol failures invalidate readiness even if the OS process has not exited yet. */
  get healthy() { return !this.failed; }

  private receive(msg: Record<string, unknown>) {
    if (msg.jsonrpc !== '2.0') throw Error('Invalid RPC version');
    if (typeof msg.method === 'string') {
      if (msg.id !== undefined) {
        // No filesystem, terminal, permission or other agent-to-client execution grants.
        this.child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: msg.id, error: { code: -32601, message: 'Unsupported client operation' } }) + '\n');
        return;
      }
      if (msg.method === 'session/update') {
        const p = record(msg.params);
        const update = record(p.update);
        if ((this.prepared.plan.harness === 'claude' || this.prepared.plan.harness === 'codex') && update.sessionUpdate === 'config_option_update') {
          if (!Array.isArray(update.configOptions)) throw Error('Invalid Claude model update');
          for (const option of update.configOptions) if (record(option).category === 'model' && record(option).currentValue !== this.prepared.plan.model) throw Error('Claude model changed');
        }
        if ((this.prepared.plan.harness === 'claude' || this.prepared.plan.harness === 'codex') && update.sessionUpdate === 'current_model_update' && update.currentModelId !== this.prepared.plan.model) throw Error('Claude model changed');
        if (this.prepared.plan.harness === 'goose') {
          if (update.sessionUpdate === 'current_model_update' && update.currentModelId !== this.prepared.plan.model) throw Error('Goose model changed');
          if (update.sessionUpdate === 'config_option_update') gooseModels(update, this.prepared.plan.model);
        }
        if (p.sessionId !== this.session || !this.prompting) return;
        if (update.sessionUpdate === 'agent_message_chunk') {
          const content = record(update.content);
          if (content.type === 'text' && typeof content.text === 'string') this.response += content.text;
        }
      }
      return;
    }
    if (typeof msg.id !== 'number') throw Error('Invalid RPC id');
    const p = this.pending.get(msg.id);
    if (!p) throw Error('Unexpected RPC response');
    this.pending.delete(msg.id); clearTimeout(p.timer);
    if (msg.error !== undefined) p.reject(Error('ACP operation rejected; repair harness setup locally (external diagnostics withheld)'));
    else p.resolve(msg.result);
  }
  private request(method: string, params: unknown): Promise<unknown> {
    if (this.failed) return Promise.reject(Error('ACP session unavailable'));
    if (this.pending.size) return Promise.reject(Error('ACP operation already in progress'));
    const id = ++this.sequence;
    return new Promise<unknown>((resolve, reject) => {
      const timer = setTimeout(() => this.fail('ACP operation timed out'), this.timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      this.child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
    }).then(value => {
      // The same stdout batch may resolve RPC then fail parsing its trailing line.
      if (this.failed) throw Error('ACP session failed before operation commit');
      return value;
    });
  }
  /** Discover a session catalog, explicitly retaining its ambiguous authentication provenance. */
  async catalog(): Promise<Catalog> {
    await this.owned.ready;
    const init = record(await this.request('initialize', { protocolVersion: this.prepared.plan.harness === 'codex' ? 2 : 1, clientCapabilities: {}, clientInfo: { name: 'beehive', version: '0.0.1' } }));
    if (init.protocolVersion !== (this.prepared.plan.harness === 'codex' ? 2 : 1) || record(init.agentInfo).name !== (this.prepared.plan.harness === 'codex' ? CODEX_ADAPTER : this.prepared.plan.harness === 'claude' ? CLAUDE_ADAPTER : this.prepared.plan.harness === 'goose' ? 'goose' : 'buzz-agent')) throw Error('Unsupported harness capabilities');
    const created = record(await this.request('session/new', { cwd: this.prepared.plan.workspace, mcpServers: [], ...(this.prepared.plan.harness === 'codex' && this.prepared.plan.instructions !== undefined ? { systemPrompt: this.prepared.plan.instructions } : {}), ...(this.prepared.plan.harness === 'claude' && this.prepared.plan.instructions !== undefined ? { _meta: { systemPrompt: { append: this.prepared.plan.instructions } } } : {}) }));
    this.session = identifier(created.sessionId);
    if (this.prepared.plan.harness === 'codex') {
      const models = codexModels(created, this.prepared.plan.model);
      this.codexModelVerified = true;
      return { state: 'reported', models, authentication: 'unverified' };
    }
    if (this.prepared.plan.harness === 'claude') return { state: 'reported', models: claudeModels(created, this.prepared.plan.model), authentication: 'unverified' };
    if (this.prepared.plan.harness === 'goose') return { state: 'reported', models: gooseModels(created, this.prepared.plan.model), authentication: 'unverified' };
    const models = record(created.models);
    if (!Array.isArray(models.availableModels) || models.availableModels.length > 1000) throw Error('Invalid ACP catalog');
    // Never publish names/descriptions/raw errors supplied by an external executable.
    const ids = [...new Set(models.availableModels.map(m => identifier(record(m).modelId)))];
    return { state: ids.length === 0 ? 'empty' : ids.includes(this.prepared.plan.model) ? 'reported' : 'filtered', models: ids, authentication: 'unverified' };
  }
  /** Require explicit exact-model acknowledgement, then a response in the SAME session/process. */
  async verify(): Promise<Evidence> {
    if (!this.session) throw Error('Create ACP session first');
    if (this.prepared.plan.harness === 'codex' && !this.codexModelVerified) throw Error('Codex exact fresh model evidence required before prompts');
    const model = this.prepared.plan.model;
    if (!this.prepared.plan.harness) {
    const ack = record(await this.request('session/set_model', { sessionId: this.session, modelId: model }));
    if (ack.sessionId !== this.session || ack.modelId !== model) throw Error('Exact-model acknowledgement mismatch');
    }
    if (this.prepared.plan.harness === 'goose' && this.prepared.plan.instructions !== undefined) await this.request('_goose/unstable/session/system-prompt/set', { sessionId: this.session, mode: 'set', key: 'buzz', text: this.prepared.plan.instructions });
    this.prompting = true;
    this.response = '';
    try {
      const reply = record(await this.request('session/prompt', { sessionId: this.session, prompt: [{ type: 'text', text: 'Reply with a short greeting only. Do not use tools or access files.' }] }));
      if (this.failed) throw Error('ACP session failed before evidence commit');
      if (reply.stopReason !== 'end_turn' || !this.response.trim()) throw Error('No completed same-session text response');
      return { session: this.session, model, responseHash: hash(this.response), stopReason: 'end_turn', source: 'external-acp-session', executableHash: this.prepared.executableHash };
    } finally { this.prompting = false; this.response = ''; }
  }
}
