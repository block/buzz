import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, existsSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { ManagerController, managerCredential, type ManagerSnapshot } from '../src/manager-controller.ts';
import { bootstrapHostIdentity } from '../src/host-identity.ts';
import { createCredential, credentialReference, readCredential, type CredentialBackend } from '../src/credential-store.ts';
import { publicKey, message, type Message } from '../src/protocol.ts';
import { profileDrafts } from '../src/profile-drafts.ts';

const secret = '1'.repeat(64), owner = publicKey(secret);
function fixture() {
  const home = mkdtempSync(join(tmpdir(), 'beehive-manager-'));
  const secrets = new Map<string,string>();
  const backend: CredentialBackend = { read: r => secrets.get(JSON.stringify(r)) ?? null, create: (r,s) => { secrets.set(JSON.stringify(r),s); }, remove: r => { secrets.delete(JSON.stringify(r)); } };
  let snapshot: ManagerSnapshot;
  let receive: (m: Message) => void = () => {};
  let closed = 0;
  const submitted: Message[] = [];
  let states: any[] = [];
  const client = { ready: Promise.resolve(), connected: true, close() { closed++; }, status: () => states, submit: (m: Message) => { submitted.push(m); }, reconcile() {} };
  const credential = async (input: any, signal: AbortSignal) => {
    signal.throwIfAborted();
    if (input.action === 'configure') { bootstrapHostIdentity(input.directory,input.label,input.owner,input.relay,backend); return { ok: true }; }
    if (input.secret) {
      if (publicKey(input.secret) !== input.owner) throw Error('mismatch');
      createCredential('owner',input.secret,backend);
    }
    return { ok: true, secret: backend.read(credentialReference('owner',input.owner)) === null ? null : readCredential(credentialReference('owner',input.owner),backend) };
  };
  const controller = new ManagerController(home, s => { snapshot = s; }, credential, ((_root: string,_url: string,_secret: string,r: typeof receive) => { receive = r; return client; }) as any);
  return { controller, home, submitted, backend, get closed() { return closed; }, get snapshot() { return snapshot!; }, receive: (m: Message) => receive(m), states: (s: any[]) => { states = s; }, cleanup() { controller.close(); rmSync(home,{ recursive: true, force: true }); } };
}

test('manager config save is real/keyless owner routing; retained configuration refuses reset; offline multiline draft persists', async () => {
  const f = fixture();
  try {
    await f.controller.request({ id: 1, action: 'configure', values: { owner, relay: 'wss://example.invalid' } });
    assert.ok(existsSync(join(f.home,'.beehive','host','host-identity.json')));
    assert.match(f.snapshot.status,/saved/);
    assert.match(f.snapshot.local[0]!.detail, /Host configured/);
    assert.ok(!f.snapshot.local[0]!.detail.includes('{'));
    assert.equal(f.snapshot.local[0]!.evidence, undefined);
    assert.equal(f.backend.read(credentialReference('owner',owner)),null);
    await f.controller.request({ id: 2, action: 'configure', values: { owner, relay: 'wss://other.invalid' } });
    assert.match(f.snapshot.status,/already exists/);
    assert.equal(f.snapshot.hostRelay,'wss://example.invalid');
    await f.controller.request({ id: 3, action: 'draft', values: { name: 'Working', instructions: 'First line\nSecond line' } });
    assert.equal(profileDrafts(join(f.home,'.beehive','owner')).list()[0]!.value.instructions,'First line\nSecond line');
  } finally { f.cleanup(); }
});

test('manager gates owner, exact selection/revision/freshness, unresolved operations and Restart handler; signout is never Stop', async () => {
  const f = fixture();
  let id = 0;
  const request = (action: string, rest = {}) => f.controller.request({ id: ++id, action, ...rest });
  try {
    await request('start'); assert.match(f.snapshot.status,/sign-in required/);
    await request('signin',{ values: { owner, relay: 'wss://example.invalid', secret: '2'.repeat(64) } }); assert.match(f.snapshot.status,/mismatch/);
    await request('signin',{ values: { owner, relay: 'wss://example.invalid', secret } }); assert.equal(f.snapshot.owner,owner);
    const m = message('inventory','host-a','agent-a',3,{ observedAt: Date.now(), phase: 'stopped', actualRun: null, assignedHost: 'host-a', configurations: { default: {} }, selectedNext: {} });
    f.receive(m);
    assert.match(f.snapshot.agents[0]!.detail, /Current run \(host report\)/);
    assert.match(f.snapshot.agents[0]!.detail, /Configuration for next start/);
    assert.ok(!f.snapshot.agents[0]!.detail.includes('{'));
    assert.equal(f.snapshot.agents[0]!.disabled!.start, '');
    const target = JSON.stringify(['host-a','agent-a']);
    await request('start',{ target, revision: 2 }); assert.match(f.snapshot.status,/selection changed/);
    await request('start',{ target, revision: 3 }); assert.equal(f.submitted.length,1); assert.equal(f.submitted[0]!.type,'start');
    await f.controller.request({ id, action: 'start', target, revision: 3 }); assert.equal(f.submitted.length,1,'duplicate request cannot act twice');
    await request('restart',{ target, revision: 3 }); assert.match(f.snapshot.status,/No Restart request is sent/); assert.equal(f.submitted.length,1,'handler refuses even if directly invoked');
    f.states([{ request: f.submitted[0], state: 'unknown' }]);
    await request('stop',{ target, revision: 3 }); assert.match(f.snapshot.status,/no confirmed result/); assert.equal(f.submitted.length,1);
    f.states([{ request: f.submitted[0], state: 'completed' }]);
    await request('select-config',{ target, revision: 3, values: { name: 'Alternative' } }); assert.equal(f.submitted[1]!.type,'save');
    await request('stop',{ target, revision: 3 }); assert.equal(f.submitted[2]!.type,'stop');
    f.receive({ ...m, revision: 4, body: { ...m.body, observedAt: Date.now()+1, phase: 'running', actualRun: { run: 'actual' } } });
    await request('start',{ target, revision: 4 }); assert.match(f.snapshot.status,/recent report from the assigned host/);
    await request('signout'); assert.equal(f.snapshot.owner,undefined); assert.equal(f.submitted.length,3); assert.equal(f.closed,1);
    f.receive(m); assert.equal(f.controller.snapshot().agents.length,0,'late receive is fenced after signout');
  } finally { f.cleanup(); }
});

test('cancelled/closed credential completion cannot sign in or open transport', async () => {
  const home = mkdtempSync(join(tmpdir(),'beehive-manager-'));
  let finish!: (v: any) => void; let connections = 0;
  const c = new ManagerController(home, () => {}, () => new Promise(resolve => { finish = resolve; }), (() => { connections++; throw Error('must not connect'); }) as any);
  try {
    const pending = c.request({ id: 1, action: 'signin', values: { owner, relay: 'wss://example.invalid' } });
    c.cancel(); c.close(); finish({ ok: true, secret }); await pending;
    assert.equal(connections,0); assert.equal(c.snapshot().owner,undefined);
    assert.ok(!existsSync(join(home,'.beehive','owner','controller.json')));
  } finally { c.close(); rmSync(home,{ recursive: true, force: true }); }
});


test('credential orchestration uses explicit Node, strips loaders, and awaits owned cancellation', async () => {
  const home = mkdtempSync(join(tmpdir(),'beehive-manager-'));
  const helper = new URL('./manager-credential-fixture.ts', import.meta.url);
  const abort = new AbortController();
  try {
    const result = await managerCredential({},abort.signal,helper);
    assert.equal(result.node,process.versions.node); assert.equal(result.overrides,undefined);
    const marker = join(home,'helper-pid');
    const pending = managerCredential({ wait: true, marker },abort.signal,helper);
    for (let i=0; i<100 && !existsSync(marker); i++) await new Promise(r => setTimeout(r,10));
    assert.ok(existsSync(marker),'owned helper reached explicit fixture boundary');
    const pid = Number(readFileSync(marker,'utf8'));
    abort.abort(); await assert.rejects(pending,/cancelled/);
    assert.throws(() => process.kill(pid,0),{ code: 'ESRCH' });
  } finally { abort.abort(); rmSync(home,{ recursive: true, force: true }); }
});

test('manager presentation preserves raw evidence, unknown states and literal configuration values', async () => {
  const f = fixture();
  try {
    await f.controller.request({ id: 1, action: 'signin', values: { owner, relay: 'wss://example.invalid', secret } });
    const m = message('inventory','host-a','agent-a',3,{ observedAt: Date.now() - 10000, phase: 'running', assignedHost: 'host-a', actualRun: { selection: { model: 'accepted', workspace: 'unconfirmed', profile: 'default' }, future: { diagnostic: 'raw detail' } }, selectedNext: { configuration: { name: 'accepted', revision: 2 }, harnessSetup: { id: 'setup', fingerprint: 'fingerprint' } } });
    f.receive(m);
    const row = f.snapshot.agents[0]!;
    assert.match(row.label, /Unknown/);
    assert.match(row.detail, /Current status unknown/);
    assert.match(row.detail, /Reported status: running/);
    assert.match(row.detail, /Model: accepted\nWorkspace: unconfirmed/);
    assert.match(row.detail, /Configuration name: accepted/);
    assert.match(row.detail, /future: diagnostic: raw detail/);
    assert.deepEqual(JSON.parse(row.evidence!), m);
    assert.match(row.disabled!.start!, /host report is old/);
    const request = message('save','host-a','agent-a',3,{ configurationAction: 'select', name: 'accepted' });
    const operation = { request, state: 'unknown', publication: 'relay policy failure; automatic retry disabled; reconcile, then retry after policy repair', result: 'new backend diagnostic' };
    f.states([operation]); f.controller.refresh();
    const shown = f.snapshot.agents.find(r => r.id === `operation:${request.id}`)!;
    assert.match(shown.label, /Choose configuration · Unknown/);
    assert.match(shown.detail, /Automatic retry is disabled/);
    assert.match(shown.detail, /Host result: new backend diagnostic/);
    assert.deepEqual(JSON.parse(shown.evidence!), operation);
    await f.controller.request({ id: 2, action: 'operations' });
    assert.match(f.snapshot.status, /Choose configuration · Unknown: new backend diagnostic/);
    await f.controller.request({ id: 3, action: 'reconcile' });
    assert.equal(f.snapshot.status, 'Checking operation results. Operations blocked by relay policy will not be retried.');
    assert.equal(f.submitted.length, 0);
    assert.equal(operation.state, 'unknown');
    f.receive(message('availability','host-b','',0,{ observedAt: Date.now(), configuration: { label: 'Host B', relay: 'wss://example.invalid' } }));
    assert.match(f.snapshot.agents.find(r => r.id === 'host:host-b')!.detail, /Host configuration: Host name: Host B/);
  } finally { f.cleanup(); }
});
