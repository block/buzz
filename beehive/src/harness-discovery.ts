import contract from './harness-contract.json' with { type: 'json' };
import { accessSync, constants, readFileSync, readdirSync, realpathSync, statSync } from 'node:fs';
import { delimiter, join } from 'node:path';
import { homedir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { spawnOwned } from './owned.ts';

/** Public capabilities, never login evidence. Non-Buzz adapters remain visible,
 * but cannot consume this catalog's OS-backed provider contract without an adapter. */
export type DetectedHarness = { id: string; label: string; executable?: string; cli?: string; state: 'available' | 'not-installed' | 'cli-missing' | 'incompatible'; providers: string[]; reason: string };
const executable = (path: string) => { try { const p = realpathSync(path); accessSync(p, constants.X_OK); return statSync(p).isFile() ? p : undefined; } catch { return undefined; } };
const version = (s: string) => { const last = s.trim().split(/\s+/).at(-1) ?? ''; return /^\d+\.\d+\.\d+$/.test(last) ? last.split('.').map(BigInt) : undefined; };
/** Desktop's strict adapter floor: unknown/prerelease/partial versions fail closed. */
export function compatibleCodex(output: string) {
  const v = version(output); if (!v) return false;
  const floor = contract.codexMinimum.map(BigInt);
  for (let i=0;i<3;i++) if (v[i] !== floor[i]) return v[i]! > floor[i]!;
  return true;
}
/** Bounded metadata probe only. Closed environment, output cap, awaited group Stop;
 * descendant-held stdout cannot extend the deadline. No auth/status subcommands. */
export async function harnessProbe(command: string, args: string[], path: string, home: string, signal: AbortSignal, deadline = 5000): Promise<string | undefined> {
  signal.throwIfAborted();
  const owned = spawnOwned(command, args, home, { PATH: path, HOME: home });
  let output = '', failed = false;
  try {
    return await new Promise<string | undefined>(resolve => {
      const finish = () => { clearTimeout(timer); signal.removeEventListener('abort', abort); resolve(failed ? undefined : output); };
      const abort = () => { failed = true; finish(); };
      const timer = setTimeout(abort, deadline);
      signal.addEventListener('abort', abort, { once: true });
      owned.child.stdout.on('data', (chunk: Buffer) => { if (Buffer.byteLength(output) + chunk.length > 4096) abort(); else output += chunk.toString(); });
      owned.child.stderr.resume();
      owned.child.on('message', (m: any) => { if (m.type === 'runner-error' || m.type === 'runner-exit') { failed ||= m.type === 'runner-error' || m.code !== 0; finish(); } });
      void owned.ready.catch(abort);
      if (signal.aborted) abort();
    });
  } finally { await owned.stop(); }
}
function nvmBin(home: string): string[] {
  const root = join(home, '.nvm'), versions = join(root, 'versions/node');
  const safe = (tag: string) => /^[a-zA-Z0-9.\-/_]+$/.test(tag) && !tag.startsWith('/') && !tag.split('/').includes('..');
  const directory = (tag: string) => { try { const p = join(versions, tag, 'bin'); return safe(tag) && statSync(p).isDirectory() ? [p] : []; } catch { return []; } };
  const read = (p: string) => { if (statSync(p).size > 4096) throw Error(); return readFileSync(p, 'utf8').trim(); };
  try { const tag = read(join(root, 'alias/default')); if (safe(tag)) { const direct = directory(tag); if (direct.length) return direct; const hop = directory(read(join(root, 'alias', tag))); if (hop.length) return hop; } } catch { /* Missing tool metadata. */ }
  try { const tags = readdirSync(versions).slice(0, 1024).filter(t => /^v\d+\.\d+\.\d+(?:-.*)?$/.test(t)); tags.sort((a,b) => { const x = version(a.slice(1).split('-')[0]!)!, y = version(b.slice(1).split('-')[0]!)!; for (let i=0;i<3;i++) if (x[i] !== y[i]) return x[i]! > y[i]! ? -1 : 1; return 0; }); return tags[0] ? directory(tags[0]) : []; } catch { return []; }
}
/** Explicit form-open discovery; no passive polling or cached stale generations.
 * Inputs permit completely isolated fixture roots and fake probe executables. */
export async function discoverHarnesses(signal: AbortSignal, options: { home?: string; path?: string; bundled?: string[]; common?: string[]; loginShells?: string[]; probe?: typeof harnessProbe } = {}): Promise<DetectedHarness[]> {
  const home = options.home ?? homedir(), path = options.path ?? process.env.PATH ?? '', probe = options.probe ?? harnessProbe;
  const bundled = options.bundled ?? [fileURLToPath(new URL('../bin', import.meta.url)), '/Applications/Buzz.app/Contents/MacOS'];
  const data = process.platform === 'darwin' ? join(home, 'Library/Application Support') : join(home, '.local/share');
  const platform = `${process.platform === 'darwin' ? 'darwin' : 'linux'}-${process.arch === 'arm64' ? 'arm64' : 'x64'}`;
  const managed = [join(data, 'Buzz/node-tools/bin'), join(data, 'Buzz/runtimes/node', contract.managedNodeVersion, platform, 'bin')];
  const common = options.common ?? ['/opt/homebrew/bin', '/usr/local/bin', '/usr/bin', '/home/linuxbrew/.linuxbrew/bin', ...['.local/share/mise/shims', '.local/bin', '.volta/bin', '.asdf/shims', '.bun/bin'].map(p => join(home,p))];
  const direct = (command: string, roots: string[]) => roots.filter(p => p.startsWith('/')).slice(0, 128).map(p => executable(join(p,command))).find(Boolean);
  let loginPath: string | undefined;
  async function resolve(command: string): Promise<string | undefined> {
    const found = direct(command, [...bundled, ...(['codex-acp','claude-agent-acp','claude-code-acp','node','npm'].includes(command) ? managed : []), ...path.split(delimiter)]); if (found) return found;
    if (loginPath === undefined) {
      loginPath = '';
      for (const shell of options.loginShells ?? ['/bin/zsh', '/bin/bash']) {
        if (!executable(shell)) continue;
        const output = await probe(shell, ['-l', '-c', 'printf "\\n%s\\n" "$PATH"'], path, home, signal, 10000);
        signal.throwIfAborted();
        if (output?.trim()) { loginPath = output.trim().split('\n').at(-1) ?? ''; break; }
      }
    }
    return direct(command, [...loginPath.split(delimiter), ...managed, ...common, ...nvmBin(home)]);
  }
  const result: DetectedHarness[] = [];
  for (const row of contract.runtimes) {
    signal.throwIfAborted();
    let binary: string | undefined;
    for (const command of row.commands) { binary = await resolve(command); if (binary) break; }
    const cli = row.underlyingCli ? await resolve(row.underlyingCli) : undefined;
    let state: DetectedHarness['state'] = !binary ? 'not-installed' : row.underlyingCli && !cli ? 'cli-missing' : 'available';
    if (state === 'available' && row.id === 'codex' && !compatibleCodex(await probe(binary!, ['--version'], [...managed, path].join(delimiter), home, signal) ?? '')) state = 'incompatible';
    signal.throwIfAborted();
    const providers = state === 'available' && row.id === 'buzz-agent' ? ['openai', 'databricks_v2'] : state === 'available' && row.id === 'codex' ? ['openai'] : [];
    const reason = state === 'not-installed' ? 'Not installed' : state === 'cli-missing' ? `Underlying ${row.underlyingCli} CLI not installed` : state === 'incompatible' ? 'Codex ACP 1.10.0 or newer required; version unknown/outdated' : providers.length ? 'Detected; provider/model access unverified' : 'Detected; this catalog’s OS-backed providers are not supported by this adapter. Use existing local setup for native authentication.';
    result.push({ id: row.id, label: row.label!, executable: binary, cli, state, providers, reason });
  }
  return result;
}
