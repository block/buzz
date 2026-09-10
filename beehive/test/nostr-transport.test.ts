import { registerHost, verifyHostRegistration } from '../src/host-registration.ts';
import test from 'node:test';
import { WebSocket } from 'ws';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, statSync, mkdirSync, rmdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { generateSecretKey, getPublicKey, finalizeEvent, getEventHash } from 'nostr-tools/pure';
import { unwrapEvent } from 'nostr-tools/nip59';
import { v2 } from 'nostr-tools/nip44';
import { message } from '../src/protocol.ts';
import { wrapManagement, unwrapManagement, authorizeManagement, type AuthenticatedMessage } from '../src/nostr-codec.ts';
import { attestHost, verifyHostAttestation } from '../src/host-attestation.ts';
import { bootstrapHostIdentity, enrollHostIdentity, readHostIdentity, requireHostEnrollment } from '../src/host-identity.ts';
import { connectNostr } from '../src/nostr-client.ts';
import { nostrFixture } from './nostr-fixture.ts';

const hex = (key: Uint8Array) => Buffer.from(key).toString('hex');
function mailbox() {
  const values: AuthenticatedMessage[] = [];
  let resolve: (() => void) | undefined;
  return { values, receive(value: AuthenticatedMessage) { values.push(value); resolve?.(); }, async count(n: number) {
    while (values.length < n) await new Promise<void>((done, reject) => {
      const timer = setTimeout(() => reject(Error('Message delivery timed out')), 2000);
      resolve = () => { clearTimeout(timer); done(); };
    });
  } };
}

test('NIP59 standard interop, tamper/wrong recipient, stable inner operation on fresh wraps', () => {
  const owner = generateSecretKey(), host = generateSecretKey();
  const operation = message('start', 'desktop', 'agent');
  const first = wrapManagement(operation, owner, getPublicKey(host));
  const second = wrapManagement(operation, owner, getPublicKey(host));
  assert.notEqual(first.id, second.id);
  assert.notEqual(first.pubkey, getPublicKey(owner));
  assert.equal(JSON.parse(unwrapEvent(first, host).content).id, operation.id);
  assert.deepEqual(unwrapManagement(first, host), { sender: getPublicKey(owner), message: operation });
  assert.deepEqual(unwrapManagement(second, host).message, operation);
  assert.throws(() => unwrapManagement(first, generateSecretKey()));
  assert.throws(() => unwrapManagement({ ...first, content: first.content.slice(0, -4) + 'AAAA' }, host));
  // Valid outer/seal signatures do not permit a different rumor author.
  const rumor = { kind: 14, pubkey: getPublicKey(host), created_at: first.created_at, content: JSON.stringify(operation), tags: [['p', getPublicKey(host)], ['subject', 'beehive-management-v1']] };
  const seal = finalizeEvent({ kind: 13, created_at: first.created_at, tags: [], content: v2.encrypt(JSON.stringify({ ...rumor, id: getEventHash(rumor) }), v2.utils.getConversationKey(owner, getPublicKey(host))) }, owner);
  const outer = generateSecretKey();
  const forged = finalizeEvent({ kind: 1059, created_at: first.created_at, tags: [['p', getPublicKey(host)]], content: v2.encrypt(JSON.stringify(seal), v2.utils.getConversationKey(outer, getPublicKey(host))) }, outer);
  assert.throws(() => unwrapManagement(forged, host), /rumor/);
});

test('independent enrolled hosts cannot forge owner, other host, or Move grant', () => {
  const owner = getPublicKey(generateSecretKey()), a = getPublicKey(generateSecretKey()), b = getPublicKey(generateSecretKey());
  const hosts = { desktop: a, laptop: b };
  assert.throws(() => authorizeManagement({ sender: a, message: message('start', 'laptop', 'agent') }, owner, hosts, { host: 'laptop' }));
  assert.throws(() => authorizeManagement({ sender: a, message: message('inventory', 'laptop', 'agent') }, owner, hosts, 'owner'));
  assert.throws(() => authorizeManagement({ sender: a, message: message('grant', 'laptop', 'agent') }, owner, hosts, { host: 'laptop' }));
  assert.throws(() => authorizeManagement({ sender: owner, message: message('start', 'desktop', 'agent') }, owner, hosts, { host: 'laptop' }));
});

test('NIP-OA SDK spec vector and AUTH time vs kind semantics', () => {
  const owner = '79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798';
  const host = 'c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5';
  const tag = ['auth', owner, 'kind=1&created_at<1713957000', '8b7df2575caf0a108374f8471722b233c53f9ff827a8b0f91861966c3b9dd5cb2e189eae9f49d72187674c2f5bd244145e10ff86c9f257ffe65a1ee5f108b369'];
  assert.deepEqual(verifyHostAttestation(tag, host, owner, 1713956999), tag);
  assert.throws(() => verifyHostAttestation(tag, host, owner, 1713957000));
  assert.throws(() => verifyHostAttestation(tag, getPublicKey(generateSecretKey()), owner, 1713956999));
});

test('durable owner-public-only pending bootstrap, genuine enrollment and installation exclusion', () => {
  const directory = mkdtempSync(join(tmpdir(), 'beehive-host-key-'));
  try {
    const owner = generateSecretKey();
    const identity = bootstrapHostIdentity(directory, 'desktop', getPublicKey(owner), 'wss://example.invalid');
    assert.equal(identity.registration, null);
    assert.notEqual(identity.secret, hex(owner));
    assert.throws(() => requireHostEnrollment(identity));
    assert.equal(statSync(join(directory, 'host-identity.json')).mode & 0o777, 0o600);
    assert.deepEqual(readHostIdentity(directory), identity);
    assert.throws(() => bootstrapHostIdentity(directory, 'other', identity.pairing.owner, 'wss://example.invalid'));
    const tag = attestHost(hex(owner), getPublicKey(Buffer.from(identity.secret, 'hex')), 'kind=1059');
    mkdirSync(join(directory, 'host.lock'));
    assert.throws(() => enrollHostIdentity(directory, tag));
    rmdirSync(join(directory, 'host.lock'));
    assert.throws(() => enrollHostIdentity(directory, attestHost(hex(generateSecretKey()), getPublicKey(Buffer.from(identity.secret, 'hex')), '')));
    assert.throws(() => enrollHostIdentity(directory, tag), 'broad OA must never enroll infrastructure');
    const registration = registerHost(identity.pairing, hex(owner), Math.floor(Date.now() / 1000) + 60);
    for (const key of ['host', 'owner', 'label', 'relay', 'nonce'] as const) {
      const changed = { ...identity.pairing, [key]: key === 'label' ? 'other' : key === 'relay' ? 'wss://other.invalid' : hex(generateSecretKey()) };
      assert.throws(() => verifyHostRegistration(registration, changed));
    }
    assert.throws(() => verifyHostRegistration({ ...registration, expires: registration.expires + 1 }, identity.pairing));
    assert.throws(() => verifyHostRegistration(registration, identity.pairing, registration.expires));
    enrollHostIdentity(directory, registration);
    assert.deepEqual(readHostIdentity(directory).registration, registration);
    assert.throws(() => requireHostEnrollment(readHostIdentity(directory)), /relay admission pending/);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

test('real Nostr wire fixture: independent owner + two hosts inventory/command/receipt, history reopen and freshwrap dedup', async () => {
  const owner = generateSecretKey(), desktop = generateSecretKey(), laptop = generateSecretKey();
  const ownerPub = getPublicKey(owner), hosts = { desktop: getPublicKey(desktop), laptop: getPublicKey(laptop) };
  const fixture = await nostrFixture(ownerPub);
  const inbox = mailbox(), desktopInbox = mailbox(), laptopInbox = mailbox();
  const clients: ReturnType<typeof connectNostr>[] = [];
  const connect = (secret: Uint8Array, box: ReturnType<typeof mailbox>) => {
    const client = connectNostr(fixture.url, secret, secret === owner ? undefined : attestHost(hex(owner), getPublicKey(secret), 'kind=1059'), box.receive, () => {});
    clients.push(client); return client;
  };
  try {
    const ui = connect(owner, inbox), a = connect(desktop, desktopInbox), b = connect(laptop, laptopInbox);
    await Promise.all([ui.ready, a.ready, b.ready]);
    await a.publish(message('inventory', 'desktop', 'agent'), ownerPub);
    await b.publish(message('inventory', 'laptop', 'agent'), ownerPub);
    await inbox.count(2);
    for (const input of inbox.values) assert.equal(authorizeManagement(input, ownerPub, hosts, 'owner').type, 'inventory');
    const operation = message('start', 'desktop', 'agent');
    await ui.publish(operation, hosts.desktop);
    await desktopInbox.count(1);
    assert.equal(laptopInbox.values.length, 0);
    assert.deepEqual(authorizeManagement(desktopInbox.values[0], ownerPub, hosts, { host: 'desktop' }), operation);
    assert.equal(inbox.values.length, 2, 'relay ACK is not a host receipt');
    await a.publish(message('receipt', 'desktop', 'agent', 0, { operation: operation.id }), ownerPub);
    await inbox.count(3);
    assert.equal(authorizeManagement(inbox.values[2], ownerPub, hosts, 'owner').body.operation, operation.id);
    a.close();
    const recovered = mailbox(); const reopen = connect(desktop, recovered); await reopen.ready;
    await recovered.count(1);
    await ui.publish(operation, hosts.desktop); await recovered.count(2);
    assert.notEqual(fixture.history[2].id, fixture.history.at(-1)?.id);
    assert.equal(new Set(recovered.values.map(v => v.message.id)).size, 1, 'durable consumer must dedup same inner id (not lifecycle proof)');
    assert.ok(fixture.independentWraps >= 5);
    const denied = connectNostr(fixture.url, generateSecretKey(), undefined, () => {}, () => {}); clients.push(denied);
    await assert.rejects(denied.ready, /authentication denied/);
  } finally { for (const client of clients) client.close(); await fixture.close(); }
});


test('fixture enforces signed AUTH, p-gated reads, supported kind, timestamp and signature admission', async () => {
  const owner = generateSecretKey();
  const fixture = await nostrFixture(getPublicKey(owner));
  const socket = new WebSocket(fixture.url);
  const frames: unknown[][] = [];
  let wake: (() => void) | undefined;
  socket.on('message', data => { frames.push(JSON.parse(data.toString())); wake?.(); });
  async function next(): Promise<unknown[]> {
    while (!frames.length) await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => reject(Error('Missing fixture response')), 2000);
      wake = () => { clearTimeout(timer); resolve(); };
    });
    return frames.shift()!;
  }
  const send = (frame: unknown[]) => socket.send(JSON.stringify(frame));
  try {
    const challenge = await next();
    const now = Math.floor(Date.now() / 1000);
    const auth = finalizeEvent({ kind: 22242, created_at: now, content: '', tags: [['relay', fixture.url], ['challenge', String(challenge[1])]] }, owner);
    send(['AUTH', { ...auth, sig: '0'.repeat(128) }]); assert.equal((await next())[2], false);
    send(['AUTH', auth]); assert.equal((await next())[2], true);
    send(['REQ', 'foreign', { kinds: [1059], '#p': [getPublicKey(generateSecretKey())] }]); assert.equal((await next())[0], 'CLOSED');
    send(['REQ', 'bare', { kinds: [1059] }]); assert.equal((await next())[0], 'CLOSED');
    const old = wrapManagement(message('start', 'desktop', 'agent'), owner, getPublicKey(generateSecretKey()), now - 901);
    send(['EVENT', old]); assert.equal((await next())[2], false);
    const unsupported = finalizeEvent({ kind: 30179, created_at: now, content: '', tags: [] }, owner);
    send(['EVENT', unsupported]); assert.equal((await next())[2], false);
    const current = wrapManagement(message('start', 'desktop', 'agent'), owner, getPublicKey(generateSecretKey()));
    send(['EVENT', { ...current, sig: '0'.repeat(128) }]); assert.equal((await next())[2], false);
    send(['EVENT', current]); assert.equal((await next())[2], true);
  } finally { socket.close(); await fixture.close(); }
});
