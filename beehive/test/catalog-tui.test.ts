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
import { privateHostTransport } from '../src/host-transport.ts';
import { host } from '../src/host.ts';
import { writePrivate } from '../src/storage.ts';
import { provisionSetup } from './provision.ts';
import { nostrFixture } from './nostr-fixture.ts';
import { connectNostr } from '../src/nostr-client.ts';
import { productionAdmission, type ScopedRelayAdmission } from '../src/relay-admission.ts';

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

test('actual owner TUI private inventory and Start/Stop use existing host executor with independent no-OA fixture admission', { timeout: 30000 }, async () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-catalog-tui-')));
  const owner = newKey(), a = newKey(), b = newKey(), agent = newKey();
  const members = new Set([owner, a, b].map(publicKey));
  const relay = await nostrFixture(publicKey(owner), members);
  const one = registration(owner, a, relay.url), two = registration(owner, b, relay.url);
  const catalog = verifyHostCatalog({ version: 1, owner: publicKey(owner), relay: relay.url, registrations: [one, two] }, publicKey(owner), relay.url);
  const admission: ScopedRelayAdmission = { admit(scope) { assert.equal(scope.relay, relay.url); assert.equal(scope.ownerDelegation, false); assert.ok(members.has(scope.publicKey)); } };
  const directory = join(root, 'host'); mkdirSync(directory);
  provisionSetup(join(directory, 'setup.json'), { host: publicKey(a), ownerSecret: owner, agentSecret: agent, runner: process.execPath, args: [resolve('test/runner.ts')], workspace: root, mode: 'fixture' });
  const running = await host(directory, relay.url, undefined, privateHostTransport(one, a, admission));
  const foreignMessages: unknown[] = [];
  const foreign = connectNostr(relay.url, Buffer.from(b, 'hex'), undefined, m => foreignMessages.push(m), () => {});
  await foreign.ready;
  const path = join(root, 'catalog.json'); writePrivate(path, catalog);
  // Replace only the admission dependency, not CLI, catalog, transport, journal or executor.
  const loader = join(root, 'admission-loader.mjs');
  writeFileSync(loader, `import { registerHooks } from 'node:module'; registerHooks({load(url,ctx,next){const r=next(url,ctx); if(!url.endsWith('/src/relay-admission.ts'))return r;return {...r,source:'export const productionAdmission = { admit(s) { if(s.relay !== '+${JSON.stringify(JSON.stringify(relay.url))}+' || s.publicKey !== '+${JSON.stringify(JSON.stringify(publicKey(owner)))}+' || s.ownerDelegation !== false) throw Error("fixture admission scope"); } };'};}});`);
  const env: NodeJS.ProcessEnv = { ...process.env }; delete env.BUZZ_PRIVATE_KEY; delete env.BUZZ_AUTH_TAG; delete env.BUZZ_RELAY_URL;
  const child = spawn(process.execPath, ['--import', loader, 'src/cli.ts', 'tui', path, relay.url], { env, stdio: ['pipe', 'pipe', 'pipe'] });
  let output = ''; child.stdout.on('data', b => output += b.toString()); child.stderr.on('data', b => output += b.toString());
  const exit = new Promise<number | null>((done, reject) => { child.on('exit', done); child.on('error', reject); });
  async function wait(predicate: () => boolean) { for (let n = 0; n < 400; n++) { if (predicate()) return; if (child.exitCode !== null) throw Error(output); await delay(20); } throw Error(`TUI observation timeout: ${output}`); }
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
    await command('start');
    await wait(() => output.includes('completed | accepted'));
    let shown = '';
    for (let n = 0; n < 80; n++) { shown = await command('show'); if (shown.includes('"phase": "running"')) break; await delay(25); }
    assert.match(shown, /"phase": "running"/);
    assert.ok(shown.includes(publicKey(agent)));
    const stopAt = output.length;
    await command('stop');
    await wait(() => output.slice(stopAt).includes('accepted'));
    for (let n = 0; n < 80; n++) { shown = await command('show'); if (shown.includes('"phase": "stopped"')) break; await delay(25); }
    assert.match(shown, /"phase": "stopped"/);
    const beforeReplay = readFileSync(join(directory, 'journal.json'), 'utf8');
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
      await command('reconcile');
      await delay(100);
      shown = await command('show');
      assert.ok(!shown.includes('FORGED'));
      assert.match(shown, /"phase": "stopped"/);
      assert.equal(readFileSync(join(directory, 'journal.json'), 'utf8'), beforeReplay);
      assert.equal(Object.keys(journal.operations).length, 2);
    } finally { ownerWire.close(); }
    assert.equal(foreignMessages.length, 0, 'another admitted host cannot read owner inventory or commands');
    assert.ok(relay.history.every(e => e.kind === 1059 && e.tags.length === 1));
    assert.ok(!output.includes(owner)); assert.ok(!output.includes(a)); assert.ok(!output.includes(agent));
    child.stdin.write('quit\n'); assert.equal(await exit, 0, output);
    const state = JSON.parse(readFileSync(join(directory, 'journal.json'), 'utf8'));
    assert.equal(state.phase, 'stopped'); assert.equal(state.revision, 2);
  } finally {
    if (child.exitCode === null && child.signalCode === null) { child.kill('SIGTERM'); await exit; }
    foreign.close(); await running.close(); await relay.close(); rmSync(root, { recursive: true, force: true });
  }
});
