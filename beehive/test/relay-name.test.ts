import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createServer } from 'node:http';
import { ManagerController, type ManagerSnapshot } from '../src/manager-controller.ts';
import { bootstrapHostIdentity } from '../src/host-identity.ts';
import { createControllerConfig } from '../src/controller-config.ts';
import { type CredentialBackend } from '../src/credential-store.ts';
import { publicKey } from '../src/protocol.ts';
import { fetchRelayName, relayNameFromDocument, relayNameHttpUrl } from '../src/relay-name.ts';

const owner = publicKey('1'.repeat(64));
const sleep = (ms: number) => new Promise(r => setTimeout(r, ms));
async function wait(fn: () => boolean) { for (let n = 0; n < 250; n++) { if (fn()) return; await sleep(20); } throw Error('Fixture wait expired'); }

test('relay name document yields only a bounded terminal-safe name or nothing', () => {
  assert.equal(relayNameFromDocument({ name: 'Fixture Relay' }), 'Fixture Relay');
  assert.equal(relayNameFromDocument({ name: '  Padded  ' }), 'Padded');
  assert.equal(relayNameFromDocument({ name: 'Södertälje Relay' }), 'Södertälje Relay');
  assert.equal(relayNameFromDocument({ description: 'no name key' }), undefined, 'absent name is not invented');
  assert.equal(relayNameFromDocument({}), undefined);
  assert.equal(relayNameFromDocument({ name: 7 }), undefined, 'non-string name is not displayed');
  assert.equal(relayNameFromDocument('{"name":"x"}'), undefined);
  assert.equal(relayNameFromDocument([{ name: 'x' }]), undefined);
  assert.equal(relayNameFromDocument(null), undefined);
  assert.equal(relayNameFromDocument({ name: 'Row\nTwo' }), undefined, 'newline is rejected, not re-flowed');
  assert.equal(relayNameFromDocument({ name: 'Carriage\rReturn' }), undefined);
  assert.equal(relayNameFromDocument({ name: 'Color\u001b[31mName' }), undefined, 'ANSI escape is rejected');
  assert.equal(relayNameFromDocument({ name: 'CSI\u009b31mName' }), undefined, 'C1 control is rejected');
  assert.equal(relayNameFromDocument({ name: 'Rtl\u202eSpoof' }), undefined, 'bidi override is rejected');
  assert.equal(relayNameFromDocument({ name: '\u2068Isolate' }), undefined);
  assert.equal(relayNameFromDocument({ name: '  \t ' }), undefined, 'whitespace-only is not a name');
  assert.equal(relayNameFromDocument({ name: 'x'.repeat(65) }), undefined, 'absurd length is rejected');
  assert.equal(relayNameFromDocument({ name: 'x'.repeat(64) }), 'x'.repeat(64), 'bounded display length is kept');
});

test('relay name HTTP URL conversion keeps the NIP-11 root and refuses non-relay or credential-bearing URLs', () => {
  assert.equal(relayNameHttpUrl('wss://relay.example'), 'https://relay.example/');
  assert.equal(relayNameHttpUrl('wss://relay.example/path'), 'https://relay.example/path');
  assert.equal(relayNameHttpUrl('ws://127.0.0.1:8080'), 'http://127.0.0.1:8080/');
  assert.equal(relayNameHttpUrl('https://plain.example'), undefined, 'already-HTTP URLs are not relay URLs');
  assert.equal(relayNameHttpUrl('ws://relay.example'), undefined, 'plain ws off loopback is not fetched');
  assert.equal(relayNameHttpUrl('wss://key:secret@relay.example'), undefined, 'credentials never become a request');
  assert.equal(relayNameHttpUrl('wss://relay.example/#fragment'), undefined);
  assert.equal(relayNameHttpUrl('not a url'), undefined);
});

test('fetchRelayName sends one unauthenticated document request and every failure stays nameless', async () => {
  const seen: { url: string; accept: string | null; authorization: string | null; signal: AbortSignal }[] = [];
  const fetcher: typeof fetch = async (url, options) => {
    const headers = new Headers(options?.headers);
    seen.push({ url: String(url), accept: headers.get('accept'), authorization: headers.get('authorization'), signal: options!.signal! });
    return new Response(JSON.stringify({ name: 'Fixture Relay', description: 'unread' }), { status: 200 });
  };
  assert.equal(await fetchRelayName('wss://name.example.invalid', new AbortController().signal, fetcher), 'Fixture Relay');
  assert.equal(seen.length, 1);
  assert.equal(seen[0].url, 'https://name.example.invalid/');
  assert.equal(seen[0].accept, 'application/nostr+json');
  assert.equal(seen[0].authorization, null, 'no credential is attached');
  const failing: Response[] = [
    new Response('', { status: 404 }),
    new Response('not json', { status: 200 }),
    new Response('{"name":7}', { status: 200 }),
    new Response('{}', { status: 200 }),
  ];
  for (const [index, response] of failing.entries()) assert.equal(await fetchRelayName('wss://fail.example.invalid', new AbortController().signal, async () => response), undefined, `case ${index} yields no name`);
  assert.equal(await fetchRelayName('wss://big.example.invalid', new AbortController().signal, async () => new Response('x'.repeat(25)), 4000, 16), undefined, 'an absurd document size yields no name');
  assert.equal(await fetchRelayName('https://not-a-relay.example.invalid', new AbortController().signal, fetcher), undefined);
  assert.equal(await fetchRelayName('wss://name.example.invalid', AbortSignal.abort(), fetcher), undefined, 'already cancelled makes no request');
  assert.equal(seen.length, 1, 'no extra requests were made');
  await assert.rejects(fetchRelayName('wss://throw.example.invalid', new AbortController().signal, async () => { throw Error('network unreachable'); }).then(() => { throw Error('resolved'); }), /resolved/);
  const aborted = new AbortController();
  const pending = fetchRelayName('wss://cancel.example.invalid', aborted.signal, (_url, options) => new Promise((_resolve, reject) => { options!.signal!.addEventListener('abort', () => reject(Error('aborted'))); }));
  await sleep(10); aborted.abort();
  assert.equal(await pending, undefined, 'caller cancellation resolves without a name');
  const start = Date.now();
  assert.equal(await fetchRelayName('wss://slow.example.invalid', new AbortController().signal, (_url, options) => new Promise((_resolve, reject) => { options!.signal!.addEventListener('abort', () => reject(Error('timeout'))); }), 40), undefined, 'bounded timeout resolves without a name');
  assert.ok(Date.now() - start < 2000, 'timeout does not wait for a slow document');
});

test('real loopback fetch reads a synthetic NIP-11 document without credentials or redirect chains', async t => {
  const requests: { method?: string; url?: string; accept?: string; authorization?: string }[] = [];
  const server = createServer((request, response) => {
    requests.push({ method: request.method, url: request.url, accept: request.headers.accept, authorization: request.headers.authorization });
    if (request.url === '/name') { response.writeHead(200, { 'Content-Type': 'application/nostr+json' }); response.end(JSON.stringify({ name: 'Loopback Fixture Relay' })); return; }
    if (request.url === '/redirect') { response.writeHead(302, { Location: 'https://elsewhere.example.invalid/' }); response.end(); return; }
    response.writeHead(500); response.end();
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  t.after(() => new Promise<void>(resolve => server.close(() => resolve())));
  const port = (server.address() as { port: number }).port;
  assert.equal(await fetchRelayName(`ws://127.0.0.1:${port}/name`, new AbortController().signal), 'Loopback Fixture Relay');
  assert.equal(requests.length, 1);
  assert.equal(requests[0].method, 'GET');
  assert.equal(requests[0].url, '/name');
  assert.equal(requests[0].accept, 'application/nostr+json');
  assert.equal(requests[0].authorization, undefined);
  assert.equal(await fetchRelayName(`ws://127.0.0.1:${port}/redirect`, new AbortController().signal), undefined, 'redirects are not followed');
  assert.equal(await fetchRelayName(`ws://127.0.0.1:${port}/`, new AbortController().signal), undefined, 'server failure keeps the URL-only footer');
  assert.equal(requests.length, 3, 'each attempt is one request');
});

function managerFixture(fetchName: (relay: string, signal: AbortSignal) => Promise<string | undefined>) {
  const home = mkdtempSync(join(tmpdir(), 'beehive-relay-name-'));
  const secrets = new Map<string, string>();
  const backend: CredentialBackend = { read: r => secrets.get(JSON.stringify(r)) ?? null, create: (r, s) => { secrets.set(JSON.stringify(r), s); }, remove: r => { secrets.delete(JSON.stringify(r)); } };
  const snapshots: ManagerSnapshot[] = [];
  const client = { ready: Promise.resolve(), connected: true, close() {}, status: () => [], submit() {}, reconcile() {} };
  const credential = async (input: any, _signal: AbortSignal) => {
    if (input.action === 'configure') { bootstrapHostIdentity(input.directory, input.label ?? 'fixture', input.owner, input.relay, backend); return { ok: true }; }
    return { ok: true, secret: null };
  };
  const controller = new ManagerController(home, s => { snapshots.push(s); }, credential, (() => client) as any, fetchName);
  return { controller, home, snapshots, get snapshot() { return snapshots.at(-1)!; }, cleanup() { controller.close(); rmSync(home, { recursive: true, force: true }); } };
}

test('controller shows the optional name for the exact configured relay; one attempt per relay URL', async () => {
  const fetches: string[] = [];
  const f = managerFixture(async relay => { fetches.push(relay); return relay === 'wss://name.example.invalid' ? 'Fixture Relay Name' : undefined; });
  try {
    assert.ok(f.snapshots.every(s => s.relayName === undefined), 'no relay name before any relay is configured');
    await f.controller.request({ id: 1, action: 'configure', values: { owner, relay: 'wss://name.example.invalid' } });
    await wait(() => f.snapshot.relayName === 'Fixture Relay Name');
    assert.equal(f.snapshot.hostRelay, 'wss://name.example.invalid');
    await wait(() => f.snapshot.service?.state === 'stopped');
    f.controller.refresh(); f.controller.refresh(); f.controller.refresh();
    await sleep(60);
    assert.equal(fetches.length, 1, 'refresh never re-fetches the same relay');
  } finally { f.cleanup(); }
});

test('name fetch failure keeps the URL-only footer and never claims disconnection', async () => {
  const f = managerFixture(async () => { throw Error('network unreachable'); });
  try {
    await f.controller.request({ id: 1, action: 'configure', values: { owner, relay: 'wss://down.example.invalid' } });
    await wait(() => f.snapshot.hostRelay === 'wss://down.example.invalid');
    await sleep(40);
    assert.equal(f.snapshot.relayName, undefined, 'no name is invented on failure');
    assert.equal(f.snapshot.service?.state, 'stopped', 'transport state is unchanged by the name failure');
    assert.equal(f.snapshot.hostRelay, 'wss://down.example.invalid');
  } finally { f.cleanup(); }
});

test('a relay switch never labels the new relay with the old name and late completion is discarded', async () => {
  const gates = new Map<string, (value: string | undefined) => void>();
  const aborted: string[] = [];
  const f = managerFixture((relay, signal) => new Promise(resolve => { gates.set(relay, resolve); signal.addEventListener('abort', () => { aborted.push(relay); }); }));
  try {
    createControllerConfig(join(f.home, '.beehive', 'owner'), owner, 'wss://old.example.invalid');
    f.controller.refresh();
    await wait(() => gates.has('wss://old.example.invalid'));
    await f.controller.request({ id: 1, action: 'configure', values: { owner, relay: 'wss://new.example.invalid' } });
    assert.equal(f.snapshot.hostRelay, 'wss://new.example.invalid');
    assert.equal(f.snapshot.relayName, undefined, 'the pending name is cleared when the relay changes');
    assert.ok(aborted.includes('wss://old.example.invalid'), 'the previous fetch is cancelled');
    gates.get('wss://old.example.invalid')!('Old Relay Name');
    await sleep(30);
    assert.equal(f.snapshot.relayName, undefined, 'a late completion for the previous relay is discarded');
    gates.get('wss://new.example.invalid')!('New Relay Name');
    await wait(() => f.snapshot.relayName === 'New Relay Name');
    assert.equal(f.snapshot.hostRelay, 'wss://new.example.invalid');
    assert.ok(f.snapshots.every(s => s.hostRelay !== 'wss://new.example.invalid' || s.relayName !== 'Old Relay Name'), 'the new relay is never labelled with the old name');
  } finally { f.cleanup(); }
});

test('closing the manager aborts the pending name fetch and publishes nothing afterwards', async () => {
  let gate!: (value: string | undefined) => void; let signal!: AbortSignal;
  const f = managerFixture((relay, s) => { signal = s; return new Promise(resolve => { gate = resolve; }); });
  try {
    await f.controller.request({ id: 1, action: 'configure', values: { owner, relay: 'wss://late.example.invalid' } });
    await wait(() => typeof gate === 'function' && f.snapshot.service?.state === 'stopped');
    const published = f.snapshots.length;
    f.controller.close();
    assert.ok(signal.aborted, 'close cancels the in-flight document read');
    gate('Late Name');
    await sleep(30);
    assert.equal(f.snapshots.length, published, 'no snapshot is published after close');
    assert.equal(f.controller.snapshot().relayName, undefined, 'a late name is never applied after close');
  } finally { f.cleanup(); }
});
