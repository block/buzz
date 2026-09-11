import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync, existsSync, realpathSync, rmSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { setTimeout as delay } from 'node:timers/promises';
import { profileDrafts } from '../src/profile-drafts.ts';
import { Profiles, profile, profileRevision } from '../src/profiles.ts';
import { message, newKey, publicKey, type Message } from '../src/protocol.ts';
import { managementClient } from '../src/intents.ts';
import { connectNostr } from '../src/nostr-client.ts';
import { hostPairing } from '../src/host-registration.ts';
import { privateHostTransport } from '../src/host-transport.ts';
import { host } from '../src/host.ts';
import { provisionSetup } from './provision.ts';
import { isolatedFileCredentials } from './isolated-file-credentials.ts';
import { nostrFixture } from './nostr-fixture.ts';

function version(instructions: string, parent: string | null = null) {
  return profile({ name: 'Careful', parent, instructions, revision: profileRevision('Careful', parent, instructions) });
}
async function until(predicate: () => boolean) {
  for (let i = 0; i < 400; i++) { if (predicate()) return; await delay(20); }
  throw Error('Private profile evidence missing');
}

test('offline CLI saves/resumes/cancels/discards drafts without signer, host or credential access', async () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-pairing-cli-private-drafts-')));
  const env = { PATH: '/usr/bin:/bin', HOME: root, BEEHIVE_TEST_CREDENTIAL_FILE: join(root, 'forbidden-credentials') };
  writeFileSync(env.BEEHIVE_TEST_CREDENTIAL_FILE, 'INVALID: any credential access must fail');
  const store = profileDrafts(root);
  async function session(run: (send: (answer: string, prompt: string) => Promise<void>, output: () => string) => Promise<void>) {
    const child = spawn(process.execPath, ['--import', resolve('test/isolated-credentials-loader.ts'), 'src/cli.ts', 'drafts', root], { env, stdio: ['pipe', 'pipe', 'pipe'] });
    let output = ''; child.stdout.on('data', b => output += b); child.stderr.on('data', b => output += b);
    const exited = new Promise<number | null>(resolve => child.on('close', resolve));
    async function send(answer: string, prompt: string) { const at = output.length; child.stdin.write(`${answer}\n`); await until(() => output.slice(at).includes(prompt)); }
    try { await until(() => { if (child.exitCode !== null) throw Error(output); return output.includes('drafts> '); }); await run(send, () => output); }
    finally { if (child.exitCode === null) child.kill('SIGTERM'); await exited; }
  }
  try {
    await session(async send => {
      await send('profile-new', 'Profile name:'); await send('Careful', 'Nonsecret behavior instructions');
      await send('unfinished but saved', 'Saved draft');
    });
    const original = store.list()[0]!; assert.equal(original.value.instructions, 'unfinished but saved');
    assert.equal(statSync(join(root, 'profile-drafts')).mode & 0o777, 0o700);
    assert.equal(statSync(join(root, 'profile-drafts', 'drafts.json')).mode & 0o777, 0o600);
    await session(async send => { await send(`profile-resume ${original.id}`, 'Nonsecret behavior instructions'); });
    assert.deepEqual(store.list(), [original], 'cancel before Enter preserves saved text');
    await session(async send => { await send(`profile-resume ${original.id}`, 'Nonsecret behavior instructions'); await send('edited text', 'Saved draft'); });
    assert.equal(store.list()[0]!.value.instructions, 'edited text');
    assert.throws(() => store.save(version('stale overwrite'), original), /changed/);
    await session(async send => { await send(`profile-discard ${original.id}`, 'Discard local draft'); await send('no', 'drafts> '); });
    assert.equal(store.list().length, 1);
    await session(async send => { await send(`profile-discard ${original.id}`, 'Discard local draft'); await send('yes', 'drafts> '); });
    assert.deepEqual(store.list(), []);
    assert.equal(existsSync(join(root, 'management-intents')), false);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('private owner library survives fresh journal/session; unrelated signer rejected; selected-next and exact Restart remain separate', async () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-pairing-cli-private-profiles-')));
  const owner = newKey(), hostSecret = newKey(), stranger = newKey(), agentSecret = newKey();
  const ownerKey = publicKey(owner), hostKey = publicKey(hostSecret), agent = publicKey(agentSecret);
  const relay = await nostrFixture(ownerKey, new Set([ownerKey, hostKey, publicKey(stranger)]));
  const catalog = { version: 1 as const, owner: ownerKey, relay: relay.url, registrations: [] };
  const a = version('private A'), b = version('private B', a.revision), foreign = version('foreign instructions');
  const profiles = new Profiles(); const seen: Message[] = [];
  let client = managementClient(join(root, 'intents-one'), relay.url, owner, m => { profiles.receive(m); seen.push(m); }, () => {}, { catalog });
  const attacker = connectNostr(relay.url, Buffer.from(stranger, 'hex'), () => {}, () => {});
  let running: Awaited<ReturnType<typeof host>> | undefined;
  try {
    await client.ready; await attacker.ready;
    const saved = profileDrafts(root).save(a);
    const op = message('profile', 'profiles', 'profiles', 0, saved.value);
    client.submit(op);
    await until(() => client.status().some(s => s.state === 'completed'));
    client.submit(message('profile', 'profiles', 'profiles', 0, b));
    await until(() => profiles.list().length === 2);
    await attacker.publish(message('profile', 'profiles', 'profiles', 0, { relay: relay.url, profile: foreign }), ownerKey);
    const ownerLibraryWire = connectNostr(relay.url, Buffer.from(owner, 'hex'), () => {}, () => {});
    await ownerLibraryWire.ready;
    try {
      await ownerLibraryWire.publish(message('profile', 'profiles', 'profiles', 0, { relay: 'wss://wrong.invalid', profile: foreign }), ownerKey);
      await ownerLibraryWire.publish(message('profile', 'profiles', 'profiles', 0, { relay: relay.url, profile: { ...foreign, instructions: 'tampered' } }), ownerKey);
    } finally { ownerLibraryWire.close(); }
    // Reopening an EMPTY local journal proves the relay, not local intents, is the library.
    client.close();
    const fresh = new Profiles();
    client = managementClient(join(root, 'intents-two'), relay.url, owner, m => { fresh.receive(m); seen.push(m); }, () => {}, { catalog });
    await client.ready;
    assert.equal(fresh.resolve(b.revision).instructions, b.instructions);
    assert.throws(() => fresh.resolve(foreign.revision), /unavailable/);
    assert.equal(client.status().length, 0);
    assert.equal(profileDrafts(root).list()[0]!.id, saved.id, 'publication does not discard saved draft');
    const directory = join(root, 'host');
    provisionSetup(join(directory, 'setup.json'), { host: hostKey, ownerPublic: ownerKey, agentSecret, runner: realpathSync(process.execPath), args: [resolve('test/runner.ts')], workspace: root, mode: 'fixture' });
    const pairing = hostPairing({ version: 1, purpose: 'beehive-host-registration', host: hostKey, owner: ownerKey, label: 'private host', relay: relay.url, nonce: newKey() });
    running = await host(directory, relay.url, undefined, privateHostTransport(pairing, hostSecret), isolatedFileCredentials(join(root, 'unused-credentials')));
    await until(() => seen.some(m => m.type === 'inventory'));
    const journal = () => JSON.parse(readFileSync(join(directory, 'journal.json'), 'utf8'));
    const selection = (p: ReturnType<typeof version>) => ({ model: 'fixture-model', workspace: root, profile: p.revision, behavior: p });
    async function request(m: Message) { client.submit(m); await until(() => client.status().some(s => s.request.id === m.id && s.state !== 'pending')); return client.status().find(s => s.request.id === m.id)!.result; }
    assert.equal(await request(message('save', hostKey, agent, 0, selection(a))), 'saved; running configuration unchanged');
    assert.equal(await request(message('start', hostKey, agent, 1)), 'accepted');
    await until(() => existsSync(join(root, 'received-instructions.jsonl')));
    const original = journal().actual;
    assert.equal(await request(message('save', hostKey, agent, 2, selection(b))), 'saved; running configuration unchanged');
    assert.deepEqual(journal().actual, original);
    assert.equal(journal().selected.behavior.instructions, b.instructions);
    const before = readFileSync(join(directory, 'journal.json'), 'utf8');
    await attacker.publish(message('save', hostKey, agent, 3, selection(foreign)), hostKey);
    // A matched authorized inspect receipt/inventory fences processing before checking bytes.
    const ownerWire = connectNostr(relay.url, Buffer.from(owner, 'hex'), () => {}, () => {}); await ownerWire.ready;
    const observation = seen.length; await ownerWire.publish(message('inspect', hostKey, agent), hostKey);
    await until(() => seen.slice(observation).some(m => m.type === 'inventory')); ownerWire.close();
    assert.equal(readFileSync(join(directory, 'journal.json'), 'utf8'), before);
    assert.match(String(await request(message('save', hostKey, agent, 3, { ...selection(b), behavior: { ...b, instructions: 'tamper' } }))), /revision conflict/);
    assert.match(String(await request(message('save', hostKey, agent, 2, selection(b)))), /revision-conflict/);
    assert.equal(await request(message('restart', hostKey, agent, 3)), 'accepted');
    await until(() => readFileSync(join(root, 'received-instructions.jsonl'), 'utf8').trim().split('\n').length === 2);
    const received = readFileSync(join(root, 'received-instructions.jsonl'), 'utf8').trim().split('\n').map(s => JSON.parse(s).instructions);
    assert.deepEqual(received, [a.instructions, b.instructions]);
    assert.equal(journal().actual.selection.behavior.revision, b.revision);
    assert.deepEqual(journal().runs[original.run], original);
    assert.equal(await request(message('stop', hostKey, agent, 4)), 'accepted');
    assert.ok(relay.history.every(e => e.kind === 1059 && !e.content.includes('private A') && !e.content.includes('private B')));
  } finally { client.close(); attacker.close(); await running?.close(); await relay.close(); rmSync(root, { recursive: true, force: true }); }
});
