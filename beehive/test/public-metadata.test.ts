import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, realpathSync, rmSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { setTimeout as delay } from 'node:timers/promises';
import { schnorr } from '@noble/curves/secp256k1';
import { finalizeEvent, verifyEvent, type Event } from 'nostr-tools/pure';
import { digest, newKey, publicKey, message, type Message } from '../src/protocol.ts';
import { publicMetadata, metadataRevision, metadataAuthorization } from '../src/public-metadata.ts';
import { metadataDrafts } from '../src/metadata-drafts.ts';
import { profile, profileRevision } from '../src/profiles.ts';
import { profileDrafts } from '../src/profile-drafts.ts';
import { host, initialState, type Setup } from '../src/host.ts';
import { createGenesis } from '../src/assignment.ts';
import { writePrivate } from '../src/storage.ts';
import { credentialReference, type CredentialBackend } from '../src/credential-store.ts';
import { privateHostTransport } from '../src/host-transport.ts';
import { hostPairing } from '../src/host-registration.ts';
import { managementClient } from '../src/intents.ts';
import { connectNostr } from '../src/nostr-client.ts';
import { nostrFixture } from './nostr-fixture.ts';

async function until(p: () => boolean) { for (let i = 0; i < 400; i++) { if (p()) return; await delay(20); } throw Error('Metadata evidence missing'); }
const value = { display_name: 'Synthetic agent', picture: 'https://example.invalid/avatar.png', about: 'Public test identity only' };
function association(owner: string, agent: string, conditions = '') { return JSON.stringify(['auth', publicKey(owner), conditions, Buffer.from(schnorr.sign(digest(`nostr:agent-auth:${agent}:${conditions}`), owner)).toString('hex')]); }

test('offline actual editor resumes public draft without network, signer or instruction mixing', async () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-pairing-cli-metadata-'))), agent = publicKey(newKey());
  const forbidden = join(root, 'forbidden'); writeFileSync(forbidden, 'INVALID: any credential read fails');
  const store = metadataDrafts(root);
  async function session(run: (send: (s: string, prompt: string) => Promise<void>) => Promise<void>) {
    const child = spawn(process.execPath, ['--import', resolve('test/isolated-credentials-loader.ts'), 'src/cli.ts', 'drafts', root], { env: { PATH: '/usr/bin:/bin', HOME: root, BEEHIVE_TEST_CREDENTIAL_FILE: forbidden }, stdio: ['pipe', 'pipe', 'pipe'] });
    let output = ''; child.stdout.on('data', b => output += b); child.stderr.on('data', b => output += b);
    const exit = new Promise(resolve => child.on('close', resolve));
    const send = async (s: string, prompt: string) => { const at = output.length; child.stdin.write(`${s}\n`); await until(() => { if (child.exitCode !== null) throw Error(output); return output.slice(at).includes(prompt); }); };
    try { await until(() => output.includes('drafts> ')); await run(send); } finally { child.kill('SIGTERM'); await exit; }
  }
  try {
    await session(async send => { await send('metadata-new', 'Public relay'); await send('wss://relay.example.invalid', 'Agent hex'); await send(agent, 'Public display'); await send(value.display_name, 'Public HTTPS'); await send(value.picture, 'Public about'); await send(value.about, 'Saved public metadata draft'); });
    const draft = store.list()[0]!; assert.deepEqual(draft.value, value);
    await session(async send => { await send(`metadata-edit ${draft.id}`, 'Public display'); await send('unfinished', 'Public HTTPS'); });
    assert.deepEqual(store.list()[0], draft, 'partial form does not tear saved snapshot');
    await session(async send => { await send(`metadata-edit ${draft.id}`, 'Public display'); await send('Edited', 'Public HTTPS'); await send('', 'Public about'); await send('', 'Saved public metadata draft'); });
    assert.deepEqual(store.list()[0]!.value, { display_name: 'Edited', picture: '', about: '' });
    assert.throws(() => store.save(draft.relay, agent, value, draft), /changed/);
    await session(async send => { await send(`metadata-discard ${draft.id}`, 'Discard local metadata'); await send('no', 'drafts> '); });
    assert.equal(store.list().length, 1);
    await session(async send => { await send(`metadata-discard ${draft.id}`, 'Discard local metadata'); await send('yes', 'drafts> '); });
    assert.equal(store.list().length, 0);
    assert.deepEqual(profileDrafts(root).list(), []);
    assert.equal(readFileSync(forbidden, 'utf8'), 'INVALID: any credential read fails');
    assert.throws(() => publicMetadata({ ...value, instructions: 'private' }), /fields/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('private owner authorizes stopped-agent kind0 with agent NIP98, exact readback, custody and stale fences', async () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-metadata-host-')));
  const owner = newKey(), hostSecret = newKey(), agentSecret = newKey(), siblingSecret = newKey(), stranger = newKey();
  const ownerKey = publicKey(owner), hostKey = publicKey(hostSecret), agent = publicKey(agentSecret), sibling = publicKey(siblingSecret);
  const authTag = association(owner, agent); const httpErrors: unknown[] = [], events: Event[] = [], authIds = new Set<string>();
  let url = '', latest: Event | undefined, deny = false, hideReadback = false, reads = 0;
  const relay = await nostrFixture(ownerKey, new Set([ownerKey, hostKey, publicKey(stranger)]), {}, (req, res) => {
    if (req.method !== 'POST') return false;
    let body = ''; req.on('data', b => body += b);
    req.on('end', () => {
      try {
        const auth = JSON.parse(Buffer.from(String(req.headers.authorization).slice(6), 'base64').toString()) as Event;
        assert(verifyEvent(auth)); assert.equal(auth.pubkey, agent); assert.equal(auth.kind, 27235);
        assert(!authIds.has(auth.id), 'native request replay protection'); authIds.add(auth.id);
        assert(auth.tags.some(t => t[0] === 'u' && t[1] === url.replace('ws:', 'http:') + req.url));
        assert(auth.tags.some(t => t[0] === 'method' && t[1] === 'POST'));
        assert(auth.tags.some(t => t[0] === 'payload' && t[1] === digest(body).toString('hex')));
        assert.equal(req.headers['x-auth-tag'], authTag); assert(schnorr.verify(JSON.parse(authTag)[3], digest(`nostr:agent-auth:${agent}:`), ownerKey)); metadataAuthorization(authTag, agent, ownerKey, auth.created_at);
        res.setHeader('Content-Type', 'application/json');
        if (req.url === '/query') { assert.deepEqual(JSON.parse(body), [{ authors: [agent], kinds: [0], limit: 1 }]); res.end(JSON.stringify(latest && !hideReadback ? [latest] : [])); }
        else {
          assert.equal(req.url, '/events'); const e = JSON.parse(body) as Event;
          assert(verifyEvent(e)); assert.equal(e.kind, 0); assert.equal(e.pubkey, agent); assert.deepEqual(e.tags, [JSON.parse(authTag)]);
          assert.deepEqual(Object.keys(JSON.parse(e.content)).sort(), ['about', 'display_name', 'picture']); assert(!body.includes('PRIVATE INSTRUCTIONS'));
          if (deny) { res.end(JSON.stringify({ event_id: e.id, accepted: false, message: 'refused' })); return; }
          latest = e; events.push(e); res.end(JSON.stringify({ event_id: e.id, accepted: true, message: 'saved' }));
        }
      } catch (e) { httpErrors.push(e); res.writeHead(400).end('{}'); }
    }); return true;
  });
  url = relay.url;
  const directory = join(root, 'host'), journalPath = (key: string) => join(directory, 'agents', key, 'journal.json');
  const harness = { runner: '/not-installed/no-provider-or-model', args: [], workspace: root, mode: 'fixture' as const, conversation: { executable: '/not-installed/no-acp', relay: url, authTag } };
  for (const [key, secret] of [[agent, agentSecret], [sibling, siblingSecret]]) {
    const setup: Setup = { ...harness, host: hostKey, ownerPublic: ownerKey, agentSecret: secret };
    const state = initialState(setup, createGenesis(ownerKey, key!, key === sibling ? publicKey(stranger) : hostKey));
    if (key === agent) {
      const behavior = profile({ name: 'Private', parent: null, instructions: 'PRIVATE INSTRUCTIONS', revision: profileRevision('Private', null, 'PRIVATE INSTRUCTIONS') });
      state.selected = { ...state.selected, profile: behavior.revision, behavior };
    }
    writePrivate(journalPath(key!), state);
  }
  writePrivate(join(directory, 'setup.json'), { version: 3, host: hostKey, ownerPublic: ownerKey, setups: { default: harness }, agents: { [agent]: { key: credentialReference('agent', agent), setup: 'default' }, [sibling]: { key: credentialReference('agent', sibling), setup: 'default' } } });
  const secrets = new Map([[agent, agentSecret], [sibling, siblingSecret]]);
  const credentials: CredentialBackend = { read(ref) { reads++; return secrets.get(ref.publicKey) ?? null; }, create() { throw Error('No credential creation'); }, remove() { throw Error('No credential removal'); } };
  const seen: Message[] = [], catalog = { version: 1 as const, owner: ownerKey, relay: url, registrations: [] };
  const client = managementClient(join(root, 'intents'), url, owner, m => seen.push(m), () => {}, { catalog });
  const attacker = connectNostr(url, Buffer.from(stranger, 'hex'), () => {}, () => {});
  let service: Awaited<ReturnType<typeof host>> | undefined;
  try {
    await client.ready; await attacker.ready;
    const pairing = hostPairing({ version: 1, purpose: 'beehive-host-registration', host: hostKey, owner: ownerKey, label: 'synthetic host', relay: url, nonce: newKey() });
    service = await host(directory, url, undefined, privateHostTransport(pairing, hostSecret), credentials);
    await until(() => seen.some(m => m.type === 'inventory' && m.agent === agent));
    const state = () => JSON.parse(readFileSync(journalPath(agent), 'utf8'));
    let other = readFileSync(journalPath(sibling), 'utf8'); const selected = state().selected;
    reads = 0;
    const draft = metadataDrafts(root).save(url, agent, value);
    const op = (v = value, revision = state().revision) => message('metadata', hostKey, agent, revision, { relay: url, value: v, revision: metadataRevision(v) });
    async function request(m: Message) { client.submit(m); await until(() => client.status().some(s => s.request.id === m.id && s.state !== 'pending')); return client.status().find(s => s.request.id === m.id)!.result; }
    assert.equal(await request({ ...op(), agent: sibling }), 'not-authority');
    assert.equal(reads, 0, 'standby assignment refuses before credential access');
    other = readFileSync(journalPath(sibling), 'utf8');
    await attacker.publish(op(), hostKey);
    assert.equal(await request({ ...op(), body: { ...op().body, relay: 'wss://wrong.invalid' } }), 'Invalid private metadata authorization/binding');
    assert.match(String(await request({ ...op(), body: { ...op().body, value: { ...value, instructions: 'PRIVATE INSTRUCTIONS' } } })), /Unexpected fields/);
    assert.equal(reads, 0); assert.equal(events.length, 0);
    const manifestBytes = readFileSync(join(directory, 'setup.json'), 'utf8');
    const invalidAssociation = JSON.parse(manifestBytes);
    invalidAssociation.setups.default.conversation.authTag = association(stranger, agent);
    writePrivate(join(directory, 'setup.json'), invalidAssociation);
    assert.match(String(await request(op())), /Invalid existing agent-owner association/);
    assert.equal(reads, 0, 'invalid retained association refuses before agent credential access');
    writeFileSync(join(directory, 'setup.json'), manifestBytes);
    const first = op(); assert.match(String(await request(first)), /^published; signed latest kind0 /);
    assert.equal(client.status().find(s => s.request.id === first.id)!.state, 'completed');
    assert.equal(reads, 1, 'only selected agent credential read'); assert.equal(events.length, 1);
    assert.equal(latest!.content, JSON.stringify(draft.value)); assert.equal(state().phase, 'stopped'); assert.equal(state().actual, null); assert.deepEqual(state().selected, selected);
    assert.equal(readFileSync(journalPath(sibling), 'utf8'), other);
    assert.equal(await request(op({ ...value, about: 'stale' }, 0)), 'revision-conflict'); assert.equal(reads, 1);
    const ownerWire = connectNostr(url, Buffer.from(owner, 'hex'), () => {}, () => {}); await ownerWire.ready;
    try { await ownerWire.publish(first, hostKey); await ownerWire.publish(message('inspect', hostKey, agent), hostKey); } finally { ownerWire.close(); }
    assert.equal(events.length, 1);
    await until(() => Math.floor(Date.now() / 1000) > state().publicMetadata.event.created_at);
    secrets.delete(agent); assert.match(String(await request(op())), /key missing/); assert.equal(events.length, 1);
    secrets.set(agent, agentSecret);
    // A valid signed same-second/future event cannot be overwritten by a guessed timestamp.
    latest = finalizeEvent({ kind: 0, created_at: Math.floor(Date.now() / 1000) + 5, tags: [JSON.parse(authTag)], content: JSON.stringify(value) }, Buffer.from(agentSecret, 'hex'));
    assert.match(String(await request(op())), /occupies this second or is future-dated/); assert.equal(events.length, 1);
    latest = events[0];
    await delay(1100); deny = true;
    assert.match(String(await request(op({ ...value, about: 'denied' }))), /did not acknowledge/); assert.equal(events.length, 1);
    // Even a rejected/unknown attempt reserves its signed second locally: a delayed
    // remote ingest must not race a new same-second event with arbitrary ID ordering.
    await until(() => Math.floor(Date.now() / 1000) > state().publicMetadata.event.created_at);
    deny = false; hideReadback = true;
    assert.match(String(await request(op({ ...value, about: 'unknown' }))), /readback differs/); assert.equal(events.length, 2);
    assert(state().publicMetadata.event.id === events[1]!.id, 'unknown candidate survives durably');
    const persisted = state().publicMetadata;
    await service.close(); service = undefined;
    service = await host(directory, url, undefined, privateHostTransport(pairing, hostSecret), credentials);
    assert.deepEqual(state().publicMetadata, persisted); assert.equal(events.length, 2, 'host reopen never silently retries unknown public side effects');
    assert.deepEqual(httpErrors, []); assert.equal(readFileSync(journalPath(sibling), 'utf8'), other);
    assert.equal(metadataDrafts(root).list()[0]!.value.about, value.about);
  } finally { client.close(); attacker.close(); await service?.close(); await relay.close(); rmSync(root, { recursive: true, force: true }); }
});

test('existing OA association rejects wrong owner, agent, conditions and forged authorization', () => {
  const owner = newKey(), agent = publicKey(newKey()), tag = association(owner, agent), now = Math.floor(Date.now() / 1000);
  assert.equal(metadataAuthorization(tag, agent, publicKey(owner), now)[1], publicKey(owner));
  assert.throws(() => metadataAuthorization(tag, publicKey(newKey()), publicKey(owner), now));
  assert.throws(() => metadataAuthorization(tag, agent, publicKey(newKey()), now));
  assert.throws(() => metadataAuthorization(association(owner, agent, 'kind=0'), agent, publicKey(owner), now));
  assert.throws(() => metadataAuthorization(association(owner, agent, 'created_at<1'), agent, publicKey(owner), now));
});
