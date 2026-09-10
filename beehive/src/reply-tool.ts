import { createServer, type Socket } from 'node:net';
import { accessSync, constants, readFileSync, realpathSync, statSync } from 'node:fs';
import { isAbsolute } from 'node:path';
import { fileURLToPath } from 'node:url';
import { randomBytes, createHash } from 'node:crypto';
import { setTimeout as delay } from 'node:timers/promises';
import { spawnOwned, type OwnedProcess } from './owned.ts';

/** Local-only, single-thread reply authority. No scope or executable comes from ACP. */
export type ReplyToolSetup = { executable: string; channel: string; parent: string; recipient: string };
/** Snapshot the explicitly provisioned CLI and reply destination before launching anything. */
export function prepareReplyTool(input: ReplyToolSetup) {
  if (!isAbsolute(input.executable) || realpathSync(input.executable) !== input.executable) throw Error('Reply CLI must be a canonical absolute path');
  accessSync(input.executable, constants.X_OK);
  if (!statSync(input.executable).isFile()) throw Error('Reply CLI must be a regular file');
  if (!/^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/.test(input.channel) || !/^[a-f0-9]{64}$/.test(input.parent) || !/^[a-f0-9]{64}$/.test(input.recipient)) throw Error('Invalid local reply scope');
  return Object.freeze({ executable: input.executable, channel: input.channel, parent: input.parent, recipient: input.recipient,
    executableHash: createHash('sha256').update(readFileSync(input.executable)).digest('hex') });
}

/** Fixed stdio MCP adapter for the existing Buzz CLI, not a message publisher.
 * The ACP harness may detach only the childless stdio shim. Every CLI process is
 * launched under a separately retained host group anchor, including on IPC loss.
 */
export class ReplyTool {
  private capability = randomBytes(32).toString('hex');
  private sockets = new Set<Socket>();
  private pids = new Set<number>();
  private owned: OwnedProcess[] = [];
  private stops = new Map<OwnedProcess, Promise<void>>();
  private current?: OwnedProcess;
  private closed = false;
  private stopping?: Promise<void>;
  private calls = 0;
  private busy = false;
  private active = false;
  private server = createServer(socket => this.accept(socket));
  readonly ready: Promise<void>;
  readonly descriptor;
  private plan; private cwd; private env; private fail;
  constructor(path: string, plan: ReturnType<typeof prepareReplyTool>, cwd: string, env: NodeJS.ProcessEnv, fail: () => void) {
    this.plan = plan; this.cwd = cwd; this.env = Object.freeze({ ...env }); this.fail = fail;
    this.descriptor = Object.freeze({ name: 'beehive-buzz-reply', command: process.execPath,
      args: [fileURLToPath(new URL('./acp-shim.ts', import.meta.url)), path, this.capability], env: [] });
    this.server.on('error', fail);
    this.ready = new Promise<void>((resolve, reject) => { this.server.once('error', reject); this.server.listen(path, resolve); });
    void this.ready.catch(fail);
  }
  /** Only a model-confirmed active ACP prompt can exercise the local reply grant. */
  setActive(active: boolean) {
    this.active = active;
    if (!active && this.current) void this.stopOwned(this.current).catch(this.fail);
  }
  private stopOwned(p: OwnedProcess) {
    let stopped = this.stops.get(p);
    if (!stopped) { stopped = p.stop(); this.stops.set(p, stopped); }
    return stopped;
  }
  private accept(socket: Socket) {
    if (this.closed || this.sockets.size >= 8) { socket.destroy(); return; }
    this.sockets.add(socket);
    let authenticated = false; let buffer = ''; let requests = 0;
    const ids = new Set<string | number>();
    const timer = setTimeout(() => socket.destroy(), 2000);
    socket.setEncoding('utf8'); socket.on('error', () => socket.destroy());
    socket.on('close', () => { clearTimeout(timer); this.sockets.delete(socket); if (authenticated && !this.closed) this.fail(); });
    const send = (id: unknown, result: unknown) => {
      if (!this.closed && !socket.destroyed && !socket.write(JSON.stringify({ jsonrpc: '2.0', id, result }) + '\n')) this.fail();
    };
    socket.on('data', (chunk: string) => {
      buffer += chunk;
      if (Buffer.byteLength(buffer) > 128 * 1024) { socket.destroy(); this.fail(); return; }
      try {
        let end: number;
        while ((end = buffer.indexOf('\n')) >= 0) {
          const m = JSON.parse(buffer.slice(0, end)); buffer = buffer.slice(end + 1);
          if (!authenticated) {
            if (m?.capability !== this.capability || !Number.isSafeInteger(m.pid) || m.pid <= 1 || this.pids.size >= 8) throw Error();
            authenticated = true; this.pids.add(m.pid); clearTimeout(timer); continue;
          }
          if (++requests > 128 || m?.jsonrpc !== '2.0' || typeof m.method !== 'string') throw Error();
          if (m.method === 'notifications/initialized' && m.id === undefined) continue;
          if (!(typeof m.id === 'number' && Number.isSafeInteger(m.id)) && !(typeof m.id === 'string' && m.id.length <= 200)) throw Error();
          if (ids.has(m.id)) throw Error();
          ids.add(m.id);
          if (m.method === 'initialize') send(m.id, { protocolVersion: '2024-11-05', capabilities: { tools: {} }, serverInfo: { name: 'beehive-buzz-reply', version: '0.0.1' } });
          else if (m.method === 'tools/list') send(m.id, { tools: [{ name: 'buzz_reply', description: 'Send an ordinary reply using the actual Buzz CLI to the locally authorized channel, parent and recipient. Destination is fixed by the host, not by tool arguments. Mention syntax (@, nostr URIs, npub/nprofile) is unsupported; the host supplies the sole explicit recipient.', inputSchema: { type: 'object', properties: { content: { type: 'string', minLength: 1, maxLength: 16384 } }, required: ['content'], additionalProperties: false } }] });
          else if (m.method === 'ping') send(m.id, {});
          else if (m.method === 'tools/call') {
            const args = m.params?.arguments;
            if (!this.active || this.busy || ++this.calls > 8 || m.params?.name !== 'buzz_reply' || !args || Object.keys(args).length !== 1 || typeof args.content !== 'string' || !args.content.trim() || Buffer.byteLength(args.content) > 16384 || /@|nostr:|npub1|nprofile1/i.test(args.content)) throw Error();
            this.busy = true;
            void this.call(args.content).then(() => send(m.id, { content: [{ type: 'text', text: 'Buzz CLI accepted the scoped reply.' }] }), () => { send(m.id, { isError: true, content: [{ type: 'text', text: 'Buzz reply failed; diagnostics withheld.' }] }); this.fail(); }).finally(() => { this.busy = false; });
          } else throw Error();
        }
      } catch { socket.destroy(); this.fail(); }
    });
  }
  private async call(content: string) {
    if (this.closed || !this.active) throw Error();
    const p = spawnOwned(this.plan.executable, ['messages', 'send', '--channel', this.plan.channel, '--reply-to', this.plan.parent, '--mention', this.plan.recipient, '--content', '-'], this.cwd, this.env);
    this.owned.push(p); this.current = p;
    let bytes = 0;
    const account = (chunk: Buffer) => { bytes += chunk.length; if (bytes > 128 * 1024) this.fail(); };
    p.child.stdout.on('data', account); p.child.stderr.on('data', account);
    p.child.stdin.on('error', () => this.fail());
    // Supervisor forwards the actual runner exit code, not its own anchor exit.
    const completion = new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => reject(Error('Reply timed out')), 10_000);
      p.child.on('message', (m: any) => { if (m?.type === 'runner-exit' || m?.type === 'runner-error') { clearTimeout(timer); m.type === 'runner-exit' && m.code === 0 ? resolve() : reject(Error('Reply CLI failed')); } });
      p.child.once('error', () => { clearTimeout(timer); reject(Error('Reply unavailable')); });
    });
    void completion.catch(() => {});
    await p.ready; p.child.stdin.end(content);
    try { await completion; } finally { this.current = undefined; }
    await this.stopOwned(p);
  }
  /** Retain all anchors through teardown; no recovered numeric PID is kill authority. */
  stop() {
    if (this.stopping) return this.stopping;
    this.closed = true; this.active = false;
    for (const socket of this.sockets) socket.destroy();
    this.stopping = (async () => {
      await this.ready.catch(() => {});
      await new Promise<void>(resolve => this.server.close(() => resolve()));
      const results = await Promise.allSettled(this.owned.map(p => this.stopOwned(p)));
      if (results.some(r => r.status === 'rejected')) throw Error('Reply tool ownership teardown failed');
      for (let i = 0; i < 100 && this.pids.size; i++) {
        for (const pid of this.pids) { try { process.kill(pid, 0); } catch (e) { if ((e as NodeJS.ErrnoException).code === 'ESRCH') this.pids.delete(pid); } }
        if (this.pids.size) await delay(25);
      }
      if (this.pids.size) throw Error('Reply tool shim exit unconfirmed');
    })();
    return this.stopping;
  }
}
