import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { hostname } from 'node:os';
import { npubEncode } from 'nostr-tools/nip19';
import { newKey, publicKey } from '../src/protocol.ts';

/** Launcher-focused smoke: the installed symlink must behave exactly like the
 * documented `node src/cli.ts` invocation from any directory, keep explicit
 * directories working, keep help public-only, and gate Node before any
 * TypeScript loading. Credential-touching journeys use the established
 * isolated credential fixture via NODE_OPTIONS, never the OS store. */
const packageRoot = fileURLToPath(new URL('..', import.meta.url));
const launcherSource = join(packageRoot, 'bin', 'beehive.cjs');
const loader = fileURLToPath(new URL('./isolated-credentials-loader.ts', import.meta.url));
const nodeBin = dirname(process.execPath);
const baseEnv = (home: string): NodeJS.ProcessEnv => ({ PATH: `${nodeBin}:/usr/bin:/bin`, HOME: home });
const isolatedEnv = (home: string, credentials: string): NodeJS.ProcessEnv => ({ ...baseEnv(home), BEEHIVE_TEST_CREDENTIAL_FILE: credentials, NODE_OPTIONS: `--import ${loader}` });

async function runLauncher(installed: string, args: string[], env: NodeJS.ProcessEnv, steps: [string, string][], cwd: string, timeoutMs = 30000): Promise<{ code: number | null; output: string; answered: number }> {
  const child = spawn(installed, args, { env, cwd, stdio: ['pipe', 'pipe', 'pipe'] });
  let output = '', cursor = 0, next = 0;
  const timer = setTimeout(() => child.kill('SIGKILL'), timeoutMs);
  child.stdout.on('data', data => {
    output += data.toString();
    if (next < steps.length) {
      const [prompt, answer] = steps[next]!;
      const index = output.indexOf(prompt, cursor);
      if (index !== -1) { cursor = index + prompt.length; next++; child.stdin.write(`${answer}\n`); }
    }
  });
  child.stderr.on('data', data => { output += data.toString(); });
  try {
    const code = await new Promise<number | null>((resolve, reject) => { child.on('error', reject); child.on('exit', resolve); });
    return { code, output: `${output}\n[answered ${next}/${steps.length} prompts]`, answered: next };
  } finally { clearTimeout(timer); child.stdin.end(); }
}

const createJourney = (owner: string): [string, string][] => [
  ['Owner PUBLIC npub', npubEncode(publicKey(owner))], ['Management relay URL (', 'wss://example.invalid'],
  ['Create host identity', 'yes'], ['Check/join community', 'no'],
];

function installInto(root: string, name = 'user bin dir'): string {
  const install = join(root, name);
  mkdirSync(install);
  const installed = join(install, 'beehive');
  symlinkSync(launcherSource, installed);
  return installed;
}

test('installed launcher prints public help from an unrelated directory with spaces and creates no state', { timeout: 20000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'beehive-pairing-cli-launcher-'));
  const home = join(root, 'home dir');
  try {
    mkdirSync(home);
    const installed = installInto(root);
    const cwd = join(root, 'unrelated cwd dir');
    mkdirSync(cwd);
    const help = await runLauncher(installed, [], baseEnv(home), [], cwd);
    assert.equal(help.code, 0, help.output);
    assert.match(help.output, /^Beehive — private host preview/, help.output);
    assert.ok(help.output.includes('setup'), help.output);
    assert.ok(help.output.includes('~/.beehive/host'), help.output);
    assert.ok(!help.output.includes('Enrollment ['), help.output);
    assert.ok(!existsSync(join(home, '.beehive')), 'help must not create state');
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('no-arg setup enrolls on the displayed default host folder via the isolated credential fixture', { timeout: 30000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'beehive-pairing-cli-launcher-'));
  const home = join(root, 'home dir');
  try {
    mkdirSync(home);
    const installed = installInto(root);
    const cwd = join(root, 'unrelated cwd dir');
    mkdirSync(cwd);
    const owner = newKey();
    const credentials = join(root, 'isolated fixture secrets.json');
    const created = await runLauncher(installed, ['setup'], isolatedEnv(home, credentials), createJourney(owner), cwd);
    assert.equal(created.code, 0, created.output);
    assert.equal(created.answered, 4, created.output);
    const folder = join(home, '.beehive', 'host');
    assert.ok(created.output.includes(`Default host folder: ${folder}`), created.output);
    const identity = JSON.parse(readFileSync(join(folder, 'host-identity.json'), 'utf8')) as { version: number; pairing: { host: string; owner: string; label: string }; registration: unknown };
    assert.equal(identity.version, 3);
    assert.equal(identity.pairing.label, hostname().slice(0, 128));
    assert.equal(identity.pairing.owner, publicKey(owner));
    assert.ok(/^[0-9a-f]{64}$/.test(identity.pairing.host));
    assert.equal(identity.registration, null);
    const fixture = readFileSync(credentials, 'utf8');
    assert.ok(fixture.includes(identity.pairing.host), 'host key stored in the isolated fixture');
    assert.equal(Object.keys(JSON.parse(fixture)).length, 1);
    const files = [join(folder, 'host-identity.json'), credentials];
    for (const value of [...files.map(file => readFileSync(file, 'utf8')), created.output]) assert.ok(!value.includes(owner), 'owner secret never persisted or printed');
    // A second default setup must refuse to replace the retained identity.
    const before = readFileSync(join(folder, 'host-identity.json'), 'utf8');
    const second = await runLauncher(installed, ['setup'], isolatedEnv(home, credentials), [
      ['Check/join community', 'no'],
    ], cwd);
    assert.equal(second.code, 0, second.output);
    assert.ok(second.output.includes('Identity, owner, relay and approval history retained'), second.output);
    assert.equal(readFileSync(join(folder, 'host-identity.json'), 'utf8'), before, 'retained identity unchanged');
    assert.ok(!existsSync(join(root, 'second request.json')));
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('explicit host directory setup is preserved through the launcher and leaves the default unused', { timeout: 30000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'beehive-pairing-cli-launcher-'));
  const home = join(root, 'home dir');
  try {
    mkdirSync(home);
    const installed = installInto(root);
    const cwd = join(root, 'unrelated cwd dir');
    mkdirSync(cwd);
    const owner = newKey();
    const explicit = join(root, 'explicit host dir');
    const credentials = join(root, 'isolated fixture secrets.json');
    const created = await runLauncher(installed, ['setup', explicit], isolatedEnv(home, credentials), createJourney(owner), cwd);
    assert.equal(created.code, 0, created.output);
    assert.ok(!created.output.includes('Default host folder:'), created.output);
    assert.ok(existsSync(join(explicit, 'host-identity.json')), 'identity lands in the explicit directory');
    assert.ok(!existsSync(join(home, '.beehive')), 'default folder untouched');
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('host startup keeps the owner-present guard on both the default folder and explicit directories', { timeout: 20000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'beehive-pairing-cli-launcher-'));
  const home = join(root, 'home dir');
  try {
    mkdirSync(home);
    const installed = installInto(root);
    const cwd = join(root, 'unrelated cwd dir');
    mkdirSync(cwd);
    const defaultFolder = join(home, '.beehive', 'host');
    mkdirSync(defaultFolder, { recursive: true });
    writeFileSync(join(defaultFolder, 'host-identity.json'), '{}', { mode: 0o600 });
    const guard = await runLauncher(installed, ['host', 'wss://example.invalid'], isolatedEnv(home, join(root, 'isolated fixture secrets.json')), [], cwd);
    assert.equal(guard.code, 1, guard.output);
    assert.ok(guard.output.includes('Private host requires --owner-present'), guard.output);
    const explicit = join(root, 'explicit host dir');
    mkdirSync(explicit);
    writeFileSync(join(explicit, 'host-identity.json'), '{}', { mode: 0o600 });
    const explicitGuard = await runLauncher(installed, ['host', explicit, 'wss://example.invalid'], isolatedEnv(home, join(root, 'isolated fixture secrets.json')), [], cwd);
    assert.equal(explicitGuard.code, 1, explicitGuard.output);
    assert.ok(explicitGuard.output.includes('Private host requires --owner-present'), explicitGuard.output);
    // Relay-first form parses the flag and resolves the default folder offline.
    const emptyHome = join(root, 'empty home dir');
    mkdirSync(emptyHome);
    const missing = await runLauncher(installed, ['host', 'ws://127.0.0.1:1', '--owner-present'], isolatedEnv(emptyHome, join(root, 'isolated fixture secrets.json')), [], cwd);
    assert.equal(missing.code, 1, missing.output);
    assert.ok(missing.output.includes(join(emptyHome, '.beehive', 'host')), missing.output);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('launcher Node gate: boundary comparison and a real old-node refusal', { timeout: 20000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'beehive-pairing-cli-launcher-'));
  const home = join(root, 'home dir');
  try {
    mkdirSync(home);
    const probe = spawnSync(process.execPath, ['-e', "const gate = require(process.argv[1]); for (const v of process.argv.slice(2)) process.stdout.write(gate.meetsMinimumNode(v, '22.18.0') + ' ');", launcherSource, '22.18.0', '22.17.9', '18.0.0', '24.15.0', '26.8.1'], { encoding: 'utf8' });
    assert.equal(probe.status, 0, probe.stderr);
    assert.equal(probe.stdout.trim(), 'true false false true true');
    // End-to-end: a PATH "node" reporting 18.9.0 must make the launcher fail
    // fast with the actionable message before any TypeScript is loaded.
    const stubBin = join(root, 'stub node bin');
    mkdirSync(stubBin);
    const patch = join(root, 'old node patch.mjs');
    writeFileSync(patch, "Object.defineProperty(process.versions, 'node', { value: '18.9.0', configurable: true });\n");
    writeFileSync(join(stubBin, 'node'), `#!/bin/sh\nexec '${process.execPath}' --import '${patch}' "$@"\n`);
    chmodSync(join(stubBin, 'node'), 0o755);
    const installed = installInto(root);
    const cwd = join(root, 'unrelated cwd dir');
    mkdirSync(cwd);
    const refused = await runLauncher(installed, [], { PATH: `${stubBin}:${nodeBin}:/usr/bin:/bin`, HOME: home }, [], cwd);
    assert.equal(refused.code, 1, refused.output);
    assert.ok(refused.output.includes('beehive: Node.js 22.18.0 or newer is required'), refused.output);
    assert.ok(refused.output.includes('18.9.0'), refused.output);
    assert.ok(refused.output.includes('nodejs.org'), refused.output);
    assert.ok(!refused.output.includes('Beehive — private host preview'), refused.output);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
