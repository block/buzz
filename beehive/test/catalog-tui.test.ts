import { isolatedFileCredentials } from './isolated-file-credentials.ts';
import { credentialReference } from '../src/credential-store.ts';
import { importSlotKey, installationSlots } from '../src/slots.ts';
import { conversationRelayFixture } from './conversation-relay-fixture.ts';
import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync, readFileSync, realpathSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { newKey, publicKey, message } from '../src/protocol.ts';
import { hostPairing, registerHost } from '../src/host-registration.ts';
import { verifyHostCatalog, catalogResponse, HostInventory } from '../src/host-catalog.ts';
import { writePrivate } from '../src/storage.ts';
import { provisionCredentialSlot } from '../src/credential-slots.ts';
import { createGenesis } from '../src/assignment.ts';
import { nostrFixture } from './nostr-fixture.ts';
import { connectNostr } from '../src/nostr-client.ts';
import { productionAdmission } from '../src/relay-admission.ts';

function registration(owner: string, key: string, relay: string, label = 'same label') {
  return registerHost(hostPairing({ version: 1, purpose: 'beehive-host-registration', host: publicKey(key), owner: publicKey(owner), label, relay, nonce: newKey() }), owner, Math.floor(Date.now() / 1000) + 600);
}

test('catalog verifies owner/signature/relay/expiry; labels cannot spoof identity and missing is not stopped', () => {
  const owner = newKey(), a = newKey(), b = newKey(), relay = 'wss://example.invalid';
  const one = registration(owner, a, relay), two = registration(owner, b, relay);
  const value = { version: 1, owner: publicKey(owner), relay, registrations: [one, two] };
  const catalog = verifyHostCatalog(value, value.owner, relay);
  assert.throws(() => verifyHostCatalog(value, publicKey(newKey()), relay));
  assert.throws(() => verifyHostCatalog(value, value.owner, 'wss://wrong.invalid'));
  assert.throws(() => verifyHostCatalog({ ...value, registrations: [{ ...one, signature: '0'.repeat(128) }] }, value.owner, relay));
  assert.throws(() => verifyHostCatalog(value, value.owner, relay, one.expires));
  const inventory = message('inventory', publicKey(a), 'agent', 0, { observedAt: Date.now(), phase: 'stopped' });
  assert.throws(() => catalogResponse(catalog, { sender: publicKey(b), message: inventory }));
  assert.throws(() => catalogResponse(catalog, { sender: publicKey(a), message: { ...inventory, host: 'same label' } }));
  const view = new HostInventory(catalog);
  assert.deepEqual(view.rows().map(r => r.status), ['unreachable/unknown', 'unreachable/unknown']);
  view.receive({ sender: publicKey(a), message: inventory });
  assert.equal(view.rows()[0]!.status, 'recent-host-report');
  assert.equal(view.rows()[0]!.report!.body.phase, 'stopped');
  assert.equal(view.rows()[1]!.status, 'unreachable/unknown');
  assert.equal(view.rows(Date.now() + 7000)[0]!.status, 'unreachable/unknown');
  assert.equal(view.rows(one.expires * 1000)[0]!.report, undefined);
  assert.throws(() => productionAdmission.admit({ relay, publicKey: publicKey(a), transport: 'nip42-nip59', ownerDelegation: false }), /pending/);
});

for (const scenario of ['external', 'installed', 'v3-conversion']) {
const installed = scenario !== 'external', converting = scenario === 'v3-conversion';
test(`actual owner TUI owner-public credential slots Start/Stop via private transport (${scenario})`, { skip: installed && !process.env.BEEHIVE_REAL_BUZZ_ACP }, async () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-pairing-cli-catalog-tui-')));
  const owner = newKey(), a = newKey(), b = newKey(), agent = newKey();
  const members = new Set([owner, a, b].map(publicKey));
  const relay = await nostrFixture(publicKey(owner), members);
  const one = registration(owner, a, relay.url), two = registration(owner, b, relay.url);
  const catalog = verifyHostCatalog({ version: 1, owner: publicKey(owner), relay: relay.url, registrations: [one, two] }, publicKey(owner), relay.url);
  const directory = join(root, 'host'); mkdirSync(directory);
  const credentialFile = join(root, 'credentials.json');
  const credentials = isolatedFileCredentials(credentialFile);
  credentials.create(credentialReference('host', publicKey(a)), a);
  writePrivate(join(directory, 'host-identity.json'), { version: 3, pairing: one.request, key: credentialReference('host', publicKey(a)), registration: one });
  const conversation = installed ? await conversationRelayFixture(root, owner, publicKey(agent)) : undefined;
  provisionCredentialSlot(directory, { host: publicKey(a), ownerPublic: publicKey(owner), runner: realpathSync(process.execPath), args: [resolve(installed ? 'test/conversation-harness-fixture.ts' : 'test/runner.ts')], workspace: root, mode: installed ? 'buzz-agent-databricks-v2' : 'fixture', ...(conversation ? { serviceHome: root, configDirectory: root, databricksHost: 'https://fixture.invalid', ...(!converting ? { conversation: { executable: realpathSync(process.env.BEEHIVE_REAL_BUZZ_ACP!), relay: conversation.url, replyTool: { executable: realpathSync(join(process.env.BEEHIVE_REAL_BUZZ_ACP!, '..', 'buzz')) } } } : {}) } : {}) }, agent, createGenesis(publicKey(owner), publicKey(agent), publicKey(a)), credentials);
  const journalPath = join(directory, 'agents', publicKey(agent), 'journal.json');
  if (converting) {
    const original = readFileSync(journalPath);
    const wizard = spawn(process.execPath, ['--import', resolve('test/isolated-credentials-loader.ts'), 'src/cli.ts', 'local-setup', directory], { env: { PATH: '/usr/bin:/bin', HOME: root, BEEHIVE_TEST_CREDENTIAL_FILE: join(root, 'unused-credentials.json') }, stdio: ['pipe', 'pipe', 'pipe'] });
    let output = ''; wizard.stdout.on('data', b => output += b); wizard.stderr.on('data', b => output += b);
    const exited = new Promise(resolve => wizard.on('close', resolve));
    try {
      for (const [prompt, answer] of [['Local action [', 'normal'], ['Existing agent public key:', publicKey(agent)], ['NEW immutable normal binding ID:', 'normal'], ['Absolute installed buzz-acp executable:', realpathSync(process.env.BEEHIVE_REAL_BUZZ_ACP!)], ['Buzz CONVERSATION relay URL', conversation!.url], ['Absolute installed buzz CLI executable:', realpathSync(join(process.env.BEEHIVE_REAL_BUZZ_ACP!, '..', 'buzz'))], ['Save NEW normal binding under installation lock', 'yes']]) {
        for (let n = 0; n < 400 && !output.includes(prompt!); n++) { if (wizard.exitCode !== null) break; await delay(20); }
        assert.ok(output.includes(prompt!), output); wizard.stdin.write(`${answer}\n`);
      }
      assert.equal(await exited, 0, output);
    } finally { if (wizard.exitCode === null && wizard.signalCode === null) wizard.kill('SIGKILL'); await exited; }
    assert.deepEqual(readFileSync(journalPath), original);
    assert.equal(installationSlots(directory, credentials)[0]!.setup.conversation, undefined);
    assert.ok(installationSlots(directory, credentials)[0]!.bindings.normal!.conversation);
  }
  const manifest = readFileSync(join(directory, 'setup.json'), 'utf8');
  assert.ok(!manifest.includes(owner) && !manifest.includes(agent));
  const helper = join(root, 'credential-child.mjs');
  writeFileSync(helper, `import { readFileSync } from 'node:fs'; let input=''; for await (const b of process.stdin) input+=b; const secret=JSON.parse(readFileSync(${JSON.stringify(credentialFile)},'utf8'))[JSON.stringify(JSON.parse(input))]; process.stdout.write(JSON.stringify(secret ? {status:'present',secret} : {status:'missing'}));`);
  const loader = join(root, 'credential-helper-loader.mjs');
  writeFileSync(loader, `import { registerHooks } from 'node:module'; registerHooks({load(url,ctx,next){const r=next(url,ctx); if(!url.endsWith('/src/credential-helper.ts'))return r;return {...r,source:String(r.source).replace("new URL('./credential-helper-child.ts', import.meta.url)", ${JSON.stringify(`new URL(${JSON.stringify('file://' + helper)})`)})};}});`);
  const hostChild = spawn(process.execPath, ['--import', loader, 'src/cli.ts', 'host', directory, relay.url, '--owner-present'], { env: { PATH: '/usr/bin:/bin', HOME: root }, stdio: ['ignore','pipe','pipe'] });
  let hostOutput = ''; hostChild.stdout.on('data', b => hostOutput += b); hostChild.stderr.on('data', b => hostOutput += b);
  const hostExit = new Promise<number | null>(resolve => hostChild.on('close', resolve));
  const running = { async close() { if (hostChild.exitCode === null && hostChild.signalCode === null) hostChild.kill('SIGTERM'); assert.equal(await hostExit, 0, hostOutput); } };
  for (let n=0; n<400 && !hostOutput.includes('Host online'); n++) { if (hostChild.exitCode !== null) break; await delay(20); }
  assert.match(hostOutput, /Host online/);
  const foreignMessages: unknown[] = [];
  const foreign = connectNostr(relay.url, Buffer.from(b, 'hex'), undefined, m => foreignMessages.push(m), () => {});
  await foreign.ready;
  const path = join(root, 'catalog.json'); writePrivate(path, catalog);
  const env: NodeJS.ProcessEnv = { ...process.env }; delete env.BUZZ_PRIVATE_KEY; delete env.BUZZ_AUTH_TAG; delete env.BUZZ_RELAY_URL;
  const child = spawn(process.execPath, ['src/cli.ts', 'tui', path, relay.url], { env, stdio: ['pipe', 'pipe', 'pipe'] });
  let output = ''; child.stdout.on('data', b => output += b.toString()); child.stderr.on('data', b => output += b.toString());
  const exit = new Promise<number | null>((done, reject) => { child.on('exit', done); child.on('error', reject); });
  async function wait(predicate: () => boolean) { for (let n = 0; n < (installed ? 3000 : 400); n++) { if (predicate()) return; if (child.exitCode !== null) throw Error(output); await delay(20); } throw Error(`TUI observation timeout: ${output}`); }
  async function command(line: string) { const start = output.length; child.stdin.write(`${line}\n`); await wait(() => output.slice(start).includes('beehive> ')); return output.slice(start); }
  try {
    await wait(() => output.includes('Owner private key'));
    child.stdin.write(`${owner}\n`);
    await wait(() => output.includes('beehive> '));
    let rows = await command('hosts');
    assert.match(rows, /UNREACHABLE \/ UNKNOWN/);
    assert.ok(rows.includes(publicKey(a))); assert.ok(rows.includes(publicKey(b)));
    assert.match(rows, /recent host report/);
    await command(`select ${publicKey(a)}`);
    if (converting) {
      await command('binding normal');
      let selected = '';
      for (let n = 0; n < 80; n++) { selected = await command('show'); if (selected.includes('"revision": 1,\n  "body"')) break; await delay(25); }
      assert.ok(selected.includes('"revision": 1,\n  "body"'), selected);
      const saved = JSON.parse(readFileSync(journalPath, 'utf8'));
      assert.equal(saved.selected.harnessSetup.id, 'normal'); assert.equal(saved.actual, null);
    }
    await command('start');
    await wait(() => output.includes('completed | accepted'));
    let shown = '';
    for (let n = 0; n < 80; n++) { shown = await command('show'); if (shown.includes('"phase": "running"')) break; await delay(25); }
    assert.match(shown, /"phase": "running"/);
    assert.ok(shown.includes(publicKey(agent)));
    if (conversation) {
      await wait(() => conversation.replies.length > 0);
      const actual = JSON.parse(readFileSync(journalPath, 'utf8')).actual;
      assert.equal(actual.evidence.agentPublicKey, publicKey(agent));
      assert.equal(actual.evidence.session, 'conversation-session');
      assert.equal(actual.selection.model, 'databricks-claude-haiku-4-5');
      console.log('New private management transport installed signed agent reply:', conversation.replies[0]!.id);
    }
    const stopAt = output.length;
    await command('stop');
    await wait(() => output.slice(stopAt).includes('accepted'));
    for (let n = 0; n < 80; n++) { shown = await command('show'); if (shown.includes('"phase": "stopped"')) break; await delay(25); }
    assert.match(shown, /"phase": "stopped"/);
    if (conversation) for (const file of ['harness-pid', 'descendant-pid', 'tool-shim-pid']) {
      const pid = Number(readFileSync(join(root, file), 'utf8'));
      assert.ok(Number.isSafeInteger(pid) && pid > 0);
      assert.throws(() => process.kill(pid, 0), (error: NodeJS.ErrnoException) => error.code === 'ESRCH', `${file} survives accepted Stop`);
    }
    const beforeReplay = readFileSync(journalPath, 'utf8');
    await foreign.publish(message('start', publicKey(a), publicKey(agent), 2), publicKey(a));
    await foreign.publish(message('inventory', publicKey(a), publicKey(agent), 999, { phase: 'FORGED', observedAt: Date.now() + 500 }), publicKey(owner));
    const ownerWire = connectNostr(relay.url, Buffer.from(owner, 'hex'), undefined, () => {}, () => {});
    await ownerWire.ready;
    try {
      const journal = JSON.parse(beforeReplay);
      // Rewrap the exact original durable Start after Stop. Relay ACK is not another execution.
      const intentDirectory = join(root, 'management-intents');
      const { readdirSync } = await import('node:fs');
      const { open } = await import('../src/protocol.ts');
      const scope = join(intentDirectory, readdirSync(intentDirectory)[0]!);
      const intent = readdirSync(scope).filter(n => n.endsWith('.intent')).map(n => JSON.parse(readFileSync(join(scope, n), 'utf8'))).map(v => open(v.envelope, owner)).find(m => m.type === 'start')!;
      await ownerWire.publish(intent, publicKey(a));
      assert.match(await command('reconcile'), /Fresh membership required/);
      await delay(100);
      shown = await command('show');
      assert.ok(!shown.includes('FORGED'));
      assert.match(shown, /"phase": "stopped"/);
      assert.equal(readFileSync(journalPath, 'utf8'), beforeReplay);
      assert.equal(Object.keys(journal.operations).length, converting ? 3 : 2);
    } finally { ownerWire.close(); }
    assert.equal(foreignMessages.length, 0, 'another admitted host cannot read owner inventory or commands');
    assert.deepEqual(relay.liveChecks.sort(), [publicKey(a), publicKey(owner)].sort());
    assert.ok(relay.history.every(e => e.kind === 1059 && e.tags.length === 1));
    assert.ok(!output.includes(owner)); assert.ok(!output.includes(a)); assert.ok(!output.includes(agent));
    const state = JSON.parse(readFileSync(journalPath, 'utf8'));
    assert.equal(state.phase, 'stopped'); assert.equal(state.revision, converting ? 3 : 2);
    // Simulate deliberate OS deletion while the host still has its old hydrated
    // setup in memory: the production pre-spawn re-read must refuse that cache.
    const reference = JSON.parse(manifest).agents[publicKey(agent)].key;
    credentials.remove(reference);
    const runs = JSON.stringify(state.runs);
    await command('start');
    await wait(() => Object.keys(JSON.parse(readFileSync(journalPath, 'utf8')).operations).length === (converting ? 4 : 3));
    const refused = JSON.parse(readFileSync(journalPath, 'utf8'));
    assert.equal(refused.phase, 'stopped'); assert.equal(refused.actual, null);
    assert.equal(JSON.stringify(refused.runs), runs);
    assert.equal(installationSlots(directory, credentials)[0]!.keyPresent, false);
    await running.close();
    importSlotKey(directory, publicKey(agent), agent, credentials);
    assert.equal(installationSlots(directory, credentials)[0]!.setup.agentSecret, agent);
    assert.equal(readFileSync(join(directory, 'setup.json'), 'utf8'), manifest);
    child.stdin.write('quit\n'); assert.equal(await exit, 0, output);

  } finally {
    if (child.exitCode === null && child.signalCode === null) { child.kill('SIGTERM'); await exit; }
    foreign.close(); await running.close(); await conversation?.close(); await relay.close(); rmSync(root, { recursive: true, force: true });
  }
});

}
