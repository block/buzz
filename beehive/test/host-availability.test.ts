import { npubEncode } from 'nostr-tools/nip19';
import { hostInviteCode, ownerPublicInput } from '../src/host-setup.ts';
import { readHostIdentityPublic } from '../src/host-identity.ts';
import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, writeFileSync, readFileSync, existsSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { newKey, publicKey, message, parseMessage } from '../src/protocol.ts';
import { bootstrapHostIdentity } from '../src/host-identity.ts';
import { credentialReference } from '../src/credential-store.ts';
import { isolatedFileCredentials } from './isolated-file-credentials.ts';
import { nostrFixture } from './nostr-fixture.ts';
import { connectNostr } from '../src/nostr-client.ts';
import { catalogTransport } from '../src/catalog-transport.ts';
import { HostOffers, HostInventory, catalogResponse, type HostCatalog } from '../src/host-catalog.ts';
import { verifyProductionAdmission } from '../src/relay-admission.ts';
import { createGenesis } from '../src/assignment.ts';
import { writePrivate } from '../src/storage.ts';

async function wait(predicate: () => boolean, detail: () => string = () => '') {
  for (let n = 0; n < 300; n++) { if (predicate()) return; await delay(20); }
  throw Error(`Timed out: ${detail()}`);
}

test('actual zero-agent host CLI advertises privately; file-free owner TUI discovery is not Start authority', async () => {
  const root = mkdtempSync(join(tmpdir(), 'beehive-pairing-cli-availability-'));
  const owner = newKey(), stranger = newKey();
  const file = join(root, 'credentials.json'), credentials = isolatedFileCredentials(file);
  const directory = join(root, 'host');
  const invites = { code: 'issued-fixture-code', policyRequired: true };
  const relay = await nostrFixture(publicKey(owner), new Set([publicKey(owner), publicKey(stranger)]), invites);
  const reads = join(root, 'reads.jsonl');
  const helper = join(root, 'credential-child.mjs');
  writeFileSync(helper, `import {readFileSync,appendFileSync} from 'node:fs';let input='';for await(const b of process.stdin)input+=b;appendFileSync(${JSON.stringify(reads)},input+'\\n');const secret=JSON.parse(readFileSync(${JSON.stringify(file)},'utf8'))[JSON.stringify(JSON.parse(input))];process.stdout.write(JSON.stringify(secret?{status:'present',secret}:{status:'missing'}));`);
  const loader = join(root, 'helper-loader.mjs');
  writeFileSync(loader, `import {registerHooks} from 'node:module';registerHooks({load(url,ctx,next){const r=next(url,ctx);if(!url.endsWith('/src/credential-helper.ts'))return r;const s=String(r.source);if(!s.includes("new URL('./credential-helper-child.ts', import.meta.url)"))throw Error('fixture injection failed');return {...r,source:s.replace("new URL('./credential-helper-child.ts', import.meta.url)",${JSON.stringify(`new URL(${JSON.stringify('file://' + helper)})`)})};}});`);
  const children: ReturnType<typeof spawn>[] = [];
  function cli(args: string[]) {
    const child = spawn(process.execPath, ['--import', resolve('test/isolated-credentials-loader.ts'), '--import', loader, 'src/cli.ts', ...args], { env: { PATH: '/usr/bin:/bin', HOME: root, BEEHIVE_TEST_CREDENTIAL_FILE: file }, stdio: ['pipe', 'pipe', 'pipe'] });
    children.push(child);
    let output = ''; child.stdout!.on('data', b => output += b); child.stderr!.on('data', b => output += b);
    const exit = new Promise<number | null>(done => child.on('close', done));
    return { child, output: () => output, exit };
  }
  async function setup(steps: [string, string][], code = 0) {
    const run = cli(['setup', directory]);
    let cursor = 0;
    for (const [prompt, answer] of steps) {
      await wait(() => run.output().slice(cursor).includes(prompt), run.output);
      cursor = run.output().length; run.child.stdin!.write(answer + '\n');
    }
    assert.equal(await run.exit, code, run.output());
    assert.ok(!run.output().includes('Host online'));
    return run.output();
  }
  await setup([['Owner PUBLIC npub', npubEncode(publicKey(owner))], ['Management relay URL', relay.url], ['Create host identity', 'yes'], ['Check/join community', 'yes'], ['Issued invite code', '']]);
  const identity = readHostIdentityPublic(directory);
  assert.equal(identity.pairing.owner, publicKey(owner));
  assert.equal(identity.registration, null);
  const retained = readFileSync(join(directory, 'host-identity.json'));
  const retainedKeys = readFileSync(file);
  // Retained unapproved f215-style state resumes without replacement or exchange.
  const rejectedPolicy = await setup([['Check/join community', 'yes'], ['Issued invite code', invites.code], ['Claim this issued invite', 'yes']], 1);
  assert.match(rejectedPolicy, /policy_acceptance_required/);
  assert.deepEqual(readFileSync(file), retainedKeys);
  invites.policyRequired = false;
  await setup([['Check/join community', 'yes'], ['Issued invite code', `http://${new URL(relay.url).host}/invite/${invites.code}`], ['Claim this issued invite', 'no']]);
  assert.equal(relay.claims.length, 1);
  const joined = await setup([['Check/join community', 'yes'], ['Issued invite code', `buzz://join?relay=${encodeURIComponent(relay.url)}&code=${invites.code}`], ['Claim this issued invite', 'yes']]);
  assert.match(joined, /membership verified now; host configured, NOT serving/);
  assert.match(joined, /beehive tui discover/);
  const member = await setup([['Check/join community', 'yes']]);
  assert.ok(!member.includes('Issued invite code'));
  assert.deepEqual(relay.claims, [identity.pairing.host, identity.pairing.host]);
  assert.deepEqual(readFileSync(join(directory, 'host-identity.json')), retained);
  assert.deepEqual(readFileSync(file), retainedKeys);
  writeFileSync(reads, ''); // Runtime-read count starts after the explicitly consented setup reads.
  const host = cli(['host', directory, '--owner-present']);
  let ownerWire: ReturnType<ReturnType<typeof catalogTransport>> | undefined;
  let wrong: ReturnType<typeof connectNostr> | undefined;
  try {
    await wait(() => host.output().includes('Host online'), host.output);
    assert.ok(!existsSync(join(directory, 'setup.json')));
    const catalog: HostCatalog = { version: 1, owner: publicKey(owner), relay: relay.url, registrations: [] };
    const reports: ReturnType<typeof message>[] = [];
    ownerWire = catalogTransport(catalog, await verifyProductionAdmission(relay.url, owner))(relay.url, owner, m => reports.push(m));
    await ownerWire.ready;
    await wait(() => reports.some(m => m.type === 'availability'));
    assert.equal(reports.filter(m => m.type === 'inventory').length, 0);
    assert.ok(relay.liveChecks.includes(identity.pairing.host));
    assert.ok(relay.history.every(e => JSON.stringify(e.tags) === JSON.stringify([['p', publicKey(owner)]])));
    assert.ok(relay.history.every(e => !e.content.includes(identity.pairing.label) && !e.content.includes(identity.pairing.host)));
    const leaked: unknown[] = [];
    wrong = connectNostr(relay.url, Buffer.from(stranger, 'hex'), undefined, m => leaked.push(m), () => {});
    await wrong.ready;
    const invalid = message('start', identity.pairing.host, publicKey(newKey()));
    await wrong.publish(invalid, identity.pairing.host);
    const start = message('start', identity.pairing.host, invalid.agent);
    ownerWire.send(start);
    await wait(() => reports.some(m => m.type === 'receipt' && m.body.operation === start.id));
    assert.equal(reports.find(m => m.body.operation === start.id)!.body.result, 'not-authority');
    await delay(150);
    assert.ok(!reports.some(m => m.body.operation === invalid.id));
    assert.equal(leaked.length, 0);
    const ui = cli(['tui', 'discover', relay.url, join(root, 'owner-state')]);
    await wait(() => ui.output().includes('Owner private key'), ui.output);
    ui.child.stdin!.write(owner + '\n');
    await wait(() => ui.output().includes('beehive> '), ui.output);
    await delay(150);
    ui.child.stdin!.write('hosts\n');
    await wait(() => ui.output().includes('available infrastructure (not agent authorization)'), ui.output);
    assert.ok(ui.output().includes(identity.pairing.host));
    ui.child.stdin!.write('agents\n');
    await wait(() => ui.output().includes('Per-host reports'), ui.output);
    assert.ok(!ui.output().includes(`Agent ${invalid.agent}`));
    ui.child.stdin!.write('quit\n'); assert.equal(await ui.exit, 0, ui.output());
    host.child.kill('SIGTERM'); assert.equal(await host.exit, 0, host.output());
    assert.deepEqual(readFileSync(join(directory, 'host-identity.json')), retained);
    assert.deepEqual(readFileSync(reads, 'utf8').trim().split('\n').map(s => JSON.parse(s)), [credentialReference('host', identity.pairing.host)]);
    assert.ok(!existsSync(join(directory, 'agents')));
    // Availability cannot repair a missing placement grant. The actual CLI must
    // reject a forged assignment before it asks the credential helper for an agent.
    const agent = newKey();
    const bindingFile = join(root, 'binding.json'), genesisFile = join(root, 'genesis.json');
    writePrivate(bindingFile, { runner: process.execPath, args: [resolve('test/runner.ts')], workspace: root, mode: 'fixture' });
    writePrivate(genesisFile, createGenesis(publicKey(owner), publicKey(agent), 'source-host'));
    const provision = cli(['provision-agent', directory, bindingFile, genesisFile]);
    await wait(() => provision.output().includes('Agent private key'), provision.output);
    provision.child.stdin!.write(agent + '\n');
    assert.equal(await provision.exit, 0, provision.output());
    assert.match(provision.output(), /Provisioned STOPPED public slot/);
    const journalFile = join(directory, 'agents', publicKey(agent), 'journal.json');
    const forged = JSON.parse(readFileSync(journalFile, 'utf8'));
    forged.assignment.assignedHost = identity.pairing.host; // No owner-authorized transfer chain.
    writePrivate(journalFile, forged);
    const rejected = cli(['host', directory, '--owner-present']);
    assert.notEqual(await rejected.exit, 0);
    assert.ok(!rejected.output().includes('Host online'));
    assert.deepEqual(JSON.parse(readFileSync(journalFile, 'utf8')), forged);
    assert.ok(readFileSync(reads, 'utf8').trim().split('\n').every(s => JSON.parse(s).role === 'host'));
    // A configured npub alone cannot pass the ordinary fresh member gate.
    const pending = bootstrapHostIdentity(join(root, 'pending'), 'Pending', publicKey(owner), relay.url, credentials);
    const denied = cli(['host', join(root, 'pending'), '--owner-present']);
    assert.notEqual(await denied.exit, 0);
    assert.match(denied.output(), /Direct membership not verified/);
    assert.ok(!denied.output().includes('Host online'));
    assert.ok(!relay.liveChecks.includes(pending.pairing.host));
    // Missing retained host credential is not repaired or regenerated by resume.
    credentials.remove(credentialReference('host', identity.pairing.host));
    const missingKeys = readFileSync(file);
    const missing = await setup([['Check/join community', 'yes']], 1);
    assert.match(missing, /Local host key missing/);
    assert.deepEqual(readFileSync(file), missingKeys);
    assert.deepEqual(readFileSync(join(directory, 'host-identity.json')), retained);

  } finally {
    ownerWire?.close(); wrong?.close();
    for (const child of children) if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
    await Promise.all(children.map(c => c.exitCode !== null || c.signalCode !== null ? Promise.resolve() : new Promise(done => c.on('close', done))));
    await relay.close(); rmSync(root, { recursive: true, force: true });
  }
});

test('availability schema and catalog fences: scope, signer, freshness, no fabricated approval', () => {
  const owner = publicKey(newKey()), host = publicKey(newKey()), relay = 'wss://example.invalid';
  const catalog: HostCatalog = { version: 1, owner, relay, registrations: [] };
  const now = Math.floor(Date.now() / 1000) * 1000;
  const configuration = { version: 1, purpose: 'beehive-host-registration', owner, host, relay, label: 'host', nonce: newKey() };
  const offer = message('availability', host, '', 0, { configuration, observedAt: now });
  const offers = new HostOffers();
  assert.throws(() => parseMessage({ ...offer, agent: 'fake-agent' }));
  assert.throws(() => catalogResponse(catalog, { sender: owner, message: offer }, now / 1000, offers));
  assert.throws(() => catalogResponse(catalog, { sender: host, message: { ...offer, body: { ...offer.body, configuration: { ...configuration, owner: host } } } }, now / 1000, offers));
  assert.throws(() => catalogResponse(catalog, { sender: host, message: offer }, now / 1000 + 7, offers));
  catalogResponse(catalog, { sender: host, message: offer }, now / 1000, offers);
  assert.equal(catalog.registrations.length, 0);
  const view = new HostInventory(catalog); view.receive({ sender: host, message: offer }, now);
  assert.equal(view.rows(now)[0]!.agent, undefined);
  assert.match(view.rows(now)[0]!.status, /available infrastructure/);
  assert.equal(view.rows(now + 7000)[0]!.status, 'unreachable/unknown');
});

test('normal public owner and issued invite inputs preserve relay binding', () => {
  const owner = publicKey(newKey());
  assert.equal(ownerPublicInput(npubEncode(owner)), owner);
  assert.equal(ownerPublicInput(owner.toUpperCase()), owner);
  assert.throws(() => ownerPublicInput('nsec1invalid'));
  assert.equal(hostInviteCode('issued-code', 'wss://example.invalid'), 'issued-code');
  assert.equal(hostInviteCode('https://example.invalid/invite/code/', 'wss://example.invalid'), 'code');
  assert.throws(() => hostInviteCode('https://other.invalid/invite/code', 'wss://example.invalid'));
  assert.throws(() => hostInviteCode('buzz://join?relay=wss://other.invalid&code=code', 'wss://example.invalid'));
});
