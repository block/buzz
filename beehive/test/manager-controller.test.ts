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
    assert.match(f.snapshot.local[0]!.detail, /Service: not checked/);
    assert.ok(!f.snapshot.local[0]!.detail.includes('{'));
    assert.match(f.snapshot.local[0]!.evidence!, /nonce/);
    assert.equal(f.backend.read(credentialReference('owner',owner)),null);
    await f.controller.request({ id: 2, action: 'configure', values: { owner, relay: 'wss://other.invalid' } });
    assert.match(f.snapshot.status,/already exists/);
    assert.match(f.snapshot.local[0]!.detail,/example.invalid/);
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
    assert.match(f.snapshot.agents[0]!.detail, /Actual run \(reported\)/);
    assert.match(f.snapshot.agents[0]!.detail, /Selected for next Start/);
    assert.ok(!f.snapshot.agents[0]!.detail.includes('{'));
    assert.equal(f.snapshot.agents[0]!.disabled!.start, '');
    const target = JSON.stringify(['host-a','agent-a']);
    await request('start',{ target, revision: 2 }); assert.match(f.snapshot.status,/Selection changed/);
    await request('start',{ target, revision: 3 }); assert.equal(f.submitted.length,1); assert.equal(f.submitted[0]!.type,'start');
    await f.controller.request({ id, action: 'start', target, revision: 3 }); assert.equal(f.submitted.length,1,'duplicate request cannot act twice');
    await request('restart',{ target, revision: 3 }); assert.match(f.snapshot.status,/Not available/); assert.equal(f.submitted.length,1,'handler refuses even if directly invoked');
    f.states([{ request: f.submitted[0], state: 'unknown' }]);
    await request('stop',{ target, revision: 3 }); assert.match(f.snapshot.status,/Unresolved/); assert.equal(f.submitted.length,1);
    f.states([{ request: f.submitted[0], state: 'completed' }]);
    await request('select-config',{ target, revision: 3, values: { name: 'Alternative' } }); assert.equal(f.submitted[1]!.type,'save');
    await request('stop',{ target, revision: 3 }); assert.equal(f.submitted[2]!.type,'stop');
    f.receive({ ...m, revision: 4, body: { ...m.body, observedAt: Date.now()+1, phase: 'running', actualRun: { run: 'actual' } } });
    await request('start',{ target, revision: 4 }); assert.match(f.snapshot.status,/fresh assigned stopped/);
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
