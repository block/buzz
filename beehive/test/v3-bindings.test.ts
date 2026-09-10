import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, writeFileSync, rmSync, realpathSync, mkdirSync, rmdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawn } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';
import { provisionCredentialSlot } from '../src/credential-slots.ts';
import { installationPublicSlots, installationSlots, addHarnessBinding, addSlot, bindingPreview, retireHarnessBinding } from '../src/slots.ts';
import { bindingFingerprint, host, validateSetup } from '../src/host.ts';
import { createGenesis } from '../src/assignment.ts';
import { newKey, publicKey, message, type Message } from '../src/protocol.ts';
import { readPrivate, writePrivate } from '../src/storage.ts';
import { isolatedFileCredentials } from './isolated-file-credentials.ts';

async function until(predicate: () => boolean) {
  for (let i = 0; i < 600; i++) { if (predicate()) return; await delay(20); }
  throw Error('Missing v3 binding evidence');
}

test('v3 actual local wizard: immutable replacement, neutral normal conversion, retired history and changed-preview fence', { timeout: 60000 }, async () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-pairing-cli-binding-')));
  const directory = join(root, 'host'), file = join(root, 'credentials.json'), credentials = isolatedFileCredentials(file);
  const owner = newKey(), secret = newKey(), agent = publicKey(secret);
  const setup = validateSetup({ host: 'v3-local', ownerPublic: publicKey(owner), mode: 'goose', runner: realpathSync(process.execPath), args: ['acp'], workspace: root, serviceHome: root, configDirectory: root, gooseProvider: 'fixture', gooseModels: ['a'] });
  async function cli(responses: [string, string, (() => void)?][], success = true, command = ['local-setup', directory]) {
    const child = spawn(process.execPath, ['--import', resolve('test/isolated-credentials-loader.ts'), 'src/cli.ts', ...command], { env: { PATH: '/usr/bin:/bin', HOME: root, BEEHIVE_TEST_CREDENTIAL_FILE: file }, stdio: ['pipe', 'pipe', 'pipe'] });
    let output = '', cursor = 0; child.stdout.on('data', b => output += b); child.stderr.on('data', b => output += b);
    const exited = new Promise(resolve => child.on('close', resolve));
    try {
      for (const [prompt, answer, action] of responses) {
        await until(() => output.indexOf(prompt, cursor) >= 0 || child.exitCode !== null);
        assert.ok(output.indexOf(prompt, cursor) >= 0, output); cursor = output.length;
        action?.(); child.stdin.write(`${answer}\n`);
      }
      assert.equal((await exited) === 0, success, output);
      assert.ok(!output.includes(secret) && !output.includes(owner)); return output;
    } finally { if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL'); await exited; }
  }
  try {
    provisionCredentialSlot(directory, setup, secret, createGenesis(publicKey(owner), agent, setup.host), credentials);
    const sibling = newKey(); addSlot(directory, sibling, createGenesis(publicKey(owner), publicKey(sibling), 'other-host'), 'default', undefined, credentials);
    const journal = join(directory, 'agents', agent, 'journal.json'), siblingJournal = join(directory, 'agents', publicKey(sibling), 'journal.json');
    const original = readFileSync(journal), siblingBytes = readFileSync(siblingJournal), keys = readFileSync(file);
    const initialManifest = readFileSync(join(directory, 'setup.json'));
    const normal = (answer: string): [string, string][] => [['Local action [', 'normal'], ['Existing agent public key:', agent], ['NEW immutable normal binding ID:', 'normal'], ['Absolute installed buzz-acp executable:', setup.runner], ['Buzz CONVERSATION relay URL', 'ws://127.0.0.1:19490'], ['Absolute installed buzz CLI executable:', setup.runner], ['Save NEW normal binding under installation lock', answer]];
    await cli(normal('no')); assert.deepEqual(readFileSync(join(directory, 'setup.json')), initialManifest);
    // A malformed synthetic credential file makes ANY read fail. Neutral wizard paths
    // must still work; the fixture loader prevents accidental native-store access.
    writeFileSync(file, 'invalid JSON');
    const genesisFile = join(root, 'exported-genesis.json');
    await cli([], true, ['assignment-export', directory, genesisFile, agent]);
    assert.deepEqual(readPrivate(genesisFile), (readPrivate(journal) as any).assignment.genesis);
    assert.match(await cli([], true, ['auth-info', directory]), /Goose/);
    assert.match(await cli([], false, ['conversation-setup', directory]), /NEW reference/);
    await cli(normal('yes'));
    const entries = installationPublicSlots(directory), fingerprint = bindingFingerprint(entries[0]!.bindings.default!);
    assert.equal(entries[0]!.bindings.default!.conversation, undefined);
    assert.ok(entries[0]!.bindings.normal!.conversation);
    await cli([['Local action [', 'replace-binding'], ['Existing binding ID to reuse:', 'default'], ['NEW immutable binding ID:', 'B'], ['Absolute compatible executable:', setup.runner], ['Allowed workspace', root], ['Retire default and replace', 'yes'], ['Save NEW binding only', 'yes']]);
    const current = installationPublicSlots(directory)[0]!;
    assert.equal(current.retiredBindings?.default, fingerprint);
    assert.equal(bindingFingerprint(current.bindings.default!), fingerprint);
    assert.equal(current.setupId, 'default');
    assert.deepEqual(readFileSync(journal), original); assert.deepEqual(readFileSync(siblingJournal), siblingBytes);
    const changed = await cli([['Local action [', 'retire-binding'], ['Existing binding ID to reuse:', 'B'], ['Retire B for new selection', 'yes', () => { const state = readPrivate(journal) as any; state.revision++; writePrivate(journal, state); }]], false);
    assert.match(changed, /Affected choices changed/);
    assert.equal(installationPublicSlots(directory)[0]!.retiredBindings?.B, undefined);
    const preview = bindingPreview(directory);
    mkdirSync(join(directory, 'host.lock'), { mode: 0o700 });
    assert.throws(() => retireHarnessBinding(directory, { id: 'B', fingerprint, confirmation: preview.confirmation }), /EEXIST/);
    rmdirSync(join(directory, 'host.lock'));
    // Restore only test fixture backend bytes; production never migrates secrets.
    writePrivate(file, JSON.parse(keys.toString()));
    assert.throws(() => addSlot(directory, newKey(), createGenesis(publicKey(owner), publicKey(newKey()), setup.host), 'default', undefined, credentials), /retired/);
    assert.equal(installationSlots(directory, credentials)[0]!.retiredBindings?.default, fingerprint);
    const manifest = readPrivate(join(directory, 'setup.json')) as any;
    manifest.retiredBindings.default = '0'.repeat(64); writePrivate(join(directory, 'setup.json'), manifest);
    assert.throws(() => installationPublicSlots(directory), /Invalid retired/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('v3 host retired binding retains history and neutral Stop; unreadable credential receipt hides local paths', async () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-pairing-cli-binding-host-'))), file = join(root, 'credentials.json');
  const credentials = isolatedFileCredentials(file), owner = newKey(), secret = newKey(), agent = publicKey(secret);
  const setup = validateSetup({ host: 'v3-host', ownerPublic: publicKey(owner), mode: 'fixture', runner: realpathSync(process.execPath), args: [resolve('test/runner.ts')], workspace: root });
  let hydration = true, reads = 0, receive!: (m: Message) => void;
  const seen: Message[] = [];
  const backend = { ...credentials, read(ref: Parameters<typeof credentials.read>[0]) { reads++; if (!hydration) throw Object.assign(Error(`EACCES: ${root}/setup.json`), { code: 'EACCES' }); return credentials.read(ref); } };
  let h: Awaited<ReturnType<typeof host>> | undefined;
  const open = () => host(root, 'ws://fixture.invalid', undefined, { binding: { host: setup.host, owner: publicKey(owner) }, validate() {}, connect(_url, _secret, callback) { receive = callback; return { ready: Promise.resolve(), send(m) { seen.push(m); }, close() {} }; } }, backend);
  async function send(type: 'start' | 'stop', revision: number) { const m = message(type, setup.host, agent, revision); receive(m); await until(() => seen.some(r => r.body.operation === m.id)); return seen.find(r => r.body.operation === m.id)!.body.result; }
  try {
    provisionCredentialSlot(root, setup, secret, createGenesis(publicKey(owner), agent, setup.host), credentials);
    h = await open(); hydration = false;
    assert.match(String(await send('start', 0)), /unreadable/); assert.ok(!JSON.stringify(seen.filter(m => m.type === 'receipt')).includes(root));
    const before = reads; assert.equal(await send('stop', 0), 'accepted'); assert.equal(reads, before);
    await h.close(); h = undefined;
    const entry = installationPublicSlots(root)[0]!;
    retireHarnessBinding(root, { id: 'default', fingerprint: bindingFingerprint(entry.setup), confirmation: bindingPreview(root).confirmation });
    hydration = true; h = await open(); hydration = false;
    assert.match(String(await send('start', 1)), /retired/);
    assert.equal((readPrivate(entry.path) as any).actual, null);
  } finally { await h?.close(); rmSync(root, { recursive: true, force: true }); }
});

test('v3 selected-next B does not change actual A until Restart; removed key Save/Stop stay neutral', async () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-pairing-cli-binding-run-'))), file = join(root, 'credentials.json');
  const credentials = isolatedFileCredentials(file), owner = newKey(), secret = newKey(), agent = publicKey(secret);
  const setup = validateSetup({ host: 'v3-run', ownerPublic: publicKey(owner), mode: 'fixture', runner: realpathSync(process.execPath), args: [resolve('test/runner.ts')], workspace: root });
  let receive!: (m: Message) => void; const seen: Message[] = [];
  let h: Awaited<ReturnType<typeof host>> | undefined;
  try {
    provisionCredentialSlot(root, setup, secret, createGenesis(publicKey(owner), agent, setup.host), credentials);
    const { host: _h, ownerPublic: _o, ...harness } = setup;
    addHarnessBinding(root, 'B', { ...harness, args: [resolve('test/binding-runner.ts')] });
    const entry = installationPublicSlots(root)[0]!, manifest = readFileSync(join(root, 'setup.json'));
    let reads = 0; const backend = { ...credentials, read(ref: Parameters<typeof credentials.read>[0]) { reads++; return credentials.read(ref); } };
    h = await host(root, 'ws://fixture.invalid', undefined, { binding: { host: setup.host, owner: publicKey(owner) }, validate() {}, connect(_url, _secret, callback) { receive = callback; return { ready: Promise.resolve(), send(m) { seen.push(m); }, close() {} }; } }, backend);
    const state = () => readPrivate(entry.path) as any;
    async function send(type: 'start' | 'stop' | 'save' | 'restart', body = {}) { const m = message(type, setup.host, agent, state().revision, body); receive(m); await until(() => seen.some(r => r.body.operation === m.id)); return seen.find(r => r.body.operation === m.id)!.body.result; }
    assert.equal(await send('start'), 'accepted'); const actualA = state().actual;
    const selection = { ...state().selected, harnessSetup: { id: 'B', fingerprint: bindingFingerprint(entry.bindings.B!) } };
    const beforeSave = reads;
    assert.equal(await send('save', selection), 'saved; running configuration unchanged'); assert.equal(reads, beforeSave);
    assert.deepEqual(state().actual, actualA); assert.equal(state().selected.harnessSetup.id, 'B');
    assert.equal(await send('restart'), 'accepted');
    assert.equal(state().actual.selection.harnessSetup.id, 'B'); assert.notEqual(state().actual.run, actualA.run);
    assert.ok(Object.values(state().runs).some((r: any) => r.run === actualA.run));
    const actualB = state().actual;
    const ref = (readPrivate(join(root, 'setup.json')) as any).agents[agent].key;
    credentials.remove(ref); // fresh synthetic store only; bypass tests pre-spawn revalidation
    assert.match(String(await send('restart')), /missing or changed/); assert.deepEqual(state().actual, actualB);
    const beforeNeutral = reads;
    assert.equal(await send('save', selection), 'saved; running configuration unchanged');
    assert.equal(await send('stop'), 'accepted'); assert.equal(reads, beforeNeutral);
    assert.match(String(await send('start')), /missing or changed/);
    assert.equal(state().actual, null); assert.equal(credentials.read(ref), null);
    assert.deepEqual(readFileSync(join(root, 'setup.json')), manifest);
  } finally { await h?.close(); rmSync(root, { recursive: true, force: true }); }
});
