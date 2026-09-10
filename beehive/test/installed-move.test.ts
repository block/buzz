import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { WebSocketServer } from 'ws';
import { mkdtempSync, realpathSync, writeFileSync, rmSync, existsSync, readFileSync, unlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createHash } from 'node:crypto';
import { schnorr } from '@noble/curves/secp256k1';
import { setTimeout as delay } from 'node:timers/promises';
import { spawn, type ChildProcess } from 'node:child_process';
import { once } from 'node:events';
import { provision, type Setup } from '../src/host.ts';
import { createGenesis } from '../src/assignment.ts';
import { relay } from '../src/relay.ts';
import { connect } from '../src/client.ts';
import { message, open, type Message } from '../src/protocol.ts';
import { newKey, publicKey } from '../src/protocol.ts';

// Explicitly opt-in installed executable; never opens an owner profile or provider.
for (const outcome of ['ok', 'reject'] as const) test(`two real hosts Move installed conversation signer: post-grant ${outcome}`,  { skip: !process.env.BEEHIVE_REAL_BUZZ_ACP }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-installed-')));
  writeFileSync(join(dir, 'mode'), 'ok');
  const secret = newKey(); const ownerSecret = newKey(); const agent = publicKey(secret); const owner = publicKey(ownerSecret);
  const channel = 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee';
  const replies: any[] = [];
  const subscriptionsBySocket = new Map<string, { socket: any; id: string }>();
  let expected: any;
  let channelReads = 0;
  let sourcePids: number[] = []; let moved = false;
  let authenticated = false; let subscriptions = 0; let delivered = false;
  function observeReply(e: any) {
    if (![9, 40002].includes(e.kind) || e.pubkey !== agent) return;
    const hash = createHash('sha256').update(JSON.stringify([0, e.pubkey, e.created_at, e.kind, e.tags, e.content])).digest('hex');
    assert.equal(e.id, hash); assert.ok(schnorr.verify(e.sig, hash, agent));
    assert.deepEqual(e.tags.filter((t: string[]) => t[0] === 'p').map((t: string[]) => t[1]), [recipient]);
    assert.ok(e.tags.some((t: string[]) => t[0] === 'h' && t[1] === expected.tags.find((t: string[]) => t[0] === 'h')[1]));
    assert.ok(e.tags.some((t: string[]) => t[0] === 'e' && t[1] === expected.id));
    assert.ok(e.content.includes('Private conversation fixture response'));
    replies.push(e); delivered = true;
  }
  function event(kind: number, tags: string[][], content: string, key = ownerSecret) {
    const pubkey = publicKey(key); const created_at = Math.floor(Date.now() / 1000);
    const id = createHash('sha256').update(JSON.stringify([0, pubkey, created_at, kind, tags, content])).digest('hex');
    return { id, pubkey, created_at, kind, tags, content, sig: Buffer.from(schnorr.sign(id, key)).toString('hex') };
  }
  const recipientKey = newKey(); const recipient = publicKey(recipientKey);
  writeFileSync(join(dir, 'recipient'), recipient);
  const outsiderKey = newKey();
  writeFileSync(join(dir, 'outsider'), publicKey(outsiderKey));
  const secondChannel = 'bbbbbbbb-cccc-4ddd-8eee-ffffffffffff';
  const members = event(39002, [['d', channel], ['p', agent], ['p', owner], ['p', recipient]], '');
  const metadata = event(39000, [['d', channel], ['name', 'fixture'], ['type', 'channel']], '');
  const inbound = event(9, [['h', channel], ['p', agent]], 'Please respond to this isolated fixture conversation.');
  const secondMembers = event(39002, [['d', secondChannel], ['p', agent], ['p', owner], ['p', recipient]], '');
  const secondMetadata = event(39000, [['d', secondChannel], ['name', 'second'], ['type', 'channel']], '');
  const second = event(9, [['h', secondChannel], ['p', agent]], 'Second isolated owner conversation.');
  const later = event(9, [['h', channel], ['p', agent], ['e', inbound.id, '', 'root']], 'Later prompt in first thread.');
  const inbounds = [inbound, second, later]; expected = inbound;
  const http = createServer(async (req, res) => {
    let body = ''; for await (const chunk of req) body += chunk;
    res.setHeader('Content-Type', 'application/json');
    if (req.url?.includes('/query')) {
      const filters = JSON.parse(body); const serialized = JSON.stringify(filters);
      if (serialized.includes('39002')) channelReads++;
      res.end(JSON.stringify(serialized.includes('39002') ? [members, secondMembers] : serialized.includes('39000') ? [metadata, secondMetadata] : inbounds.filter(e => serialized.includes(e.id))));
    } else if (req.url?.includes('/events')) {
      const e = JSON.parse(body); observeReply(e);
      res.end(JSON.stringify({ id: e.id, accepted: true }));
    } else res.end('[]');
  });
  const ws = new WebSocketServer({ server: http });
  ws.on('connection', socket => {
    socket.send(JSON.stringify(['AUTH', 'beehive-isolated-challenge']));
    socket.on('message', bytes => {
      const m = JSON.parse(bytes.toString());
      if (m[0] === 'AUTH') {
        const e = m[1];
        assert.equal(e.pubkey, agent); assert.equal(e.kind, 22242);
        assert.ok(e.tags.some((t: string[]) => t[0] === 'challenge' && t[1] === 'beehive-isolated-challenge'));
        const hash = createHash('sha256').update(JSON.stringify([0, e.pubkey, e.created_at, e.kind, e.tags, e.content])).digest('hex');
        assert.equal(e.id, hash); assert.ok(schnorr.verify(e.sig, hash, e.pubkey));
        if (moved) for (const pid of sourcePids) assert.throws(() => process.kill(pid, 0), (e: any) => e.code === 'ESRCH', 'complete source exit precedes target identity authentication');
        authenticated = true; socket.send(JSON.stringify(['OK', e.id, true, '']));
      } else if (m[0] === 'EVENT') { observeReply(m[1]); socket.send(JSON.stringify(['OK', m[1].id, true, ''])); }
      else if (m[0] === 'REQ') {
        subscriptions++; socket.send(JSON.stringify(['EOSE', m[1]]));
        for (const f of m.slice(2)) for (const c of f['#h'] ?? []) { subscriptionsBySocket.set(c, { socket, id: m[1] }); }
        if (m.slice(2).some((f: any) => f['#h']?.includes(channel))) setTimeout(() => socket.send(JSON.stringify(['EVENT', m[1], expected])), 100);
      }
    });
  });
  await new Promise<void>(resolve => http.listen(0, '127.0.0.1', resolve));
  const address = http.address(); assert.ok(address && typeof address !== 'string');
  const management = await relay(0, owner, join(dir, 'management-relay.json'));
  const managementAddress = management.address(); assert.ok(managementAddress && typeof managementAddress !== 'string');
  const url = `ws://127.0.0.1:${managementAddress.port}`;
  const seen: Message[] = [], children: ChildProcess[] = [];
  const ui = connect(url, ownerSecret, m => seen.push(m)); await ui.ready;
  const journal = (name: string) => JSON.parse(readFileSync(join(dir, name, 'journal.json'), 'utf8'));
  async function wait(predicate: () => boolean) {
    for (let i = 0; i < 3000; i++) { if (predicate()) return; await delay(10); }
    throw Error('Missing installed Move evidence: ' + JSON.stringify({ source: journal('source').phase, target: journal('target').phase, operations: Object.values(journal('target').operations).map((op: any) => op.reply.body.result), incoming: Object.values(journal('target').incoming ?? {}).map((m: any) => m.body.result) }));
  }
  async function request(type: Message['type'], name: string, revision: number, body = {}) {
    const m = message(type, name, agent, revision, body); ui.send(m);
    await wait(() => seen.some(r => r.type === 'receipt' && r.host === name && r.body.operation === m.id));
    return seen.find(r => r.type === 'receipt' && r.host === name && r.body.operation === m.id)!;
  }
  const genesis = createGenesis(owner, agent, 'source');
  for (const name of ['source', 'target']) {
    const setup: Setup = { host: name, ownerSecret, agentSecret: secret,
      runner: realpathSync(process.execPath), args: [resolve('test/conversation-harness-fixture.ts')], workspace: dir,
      serviceHome: dir, configDirectory: dir, databricksHost: 'https://fixture.invalid', mode: 'buzz-agent-databricks-v2',
      conversation: { executable: realpathSync(process.env.BEEHIVE_REAL_BUZZ_ACP!), relay: `ws://127.0.0.1:${address.port}`,
        replyTool: { executable: realpathSync(join(process.env.BEEHIVE_REAL_BUZZ_ACP!, '..', 'buzz')) } } };
    provision(join(dir, name), setup, genesis);
    const child = spawn(process.execPath, ['src/cli.ts', 'host', join(dir, name), url], { stdio: ['ignore', 'pipe', 'pipe'] });
    children.push(child); child.stdout?.resume(); child.stderr?.resume();
    await wait(() => seen.some(m => m.type === 'inventory' && m.host === name));
  }
  try {
    assert.equal((await request('start', 'source', 0)).body.result, 'accepted');
    for (let i = 0; i < 100 && !subscriptions; i++) await delay(50);
    assert.ok(authenticated, 'installed executable must authenticate with the provisioned key');
    assert.ok(subscriptions > 0, 'installed executable must enter subscription loop');
    const evidence = journal('source').actual.evidence;
    assert.equal(evidence.session, 'conversation-session');
    assert.equal(evidence.agentPublicKey, agent);
    assert.ok(delivered, 'the actual Buzz CLI must publish a signed threaded agent reply, not just return tool JSON');
    sourcePids = ['harness-pid', 'descendant-pid', 'tool-shim-pid'].map(file => Number(readFileSync(join(dir, file), 'utf8')));
    assert.ok(sourcePids.every(pid => Number.isSafeInteger(pid) && pid > 0));
    const selected = { model: 'databricks-claude-haiku-4-5', workspace: dir, profile: 'default' };
    assert.match(String((await request('move', 'source', 1, { target: 'target', targetRevision: 0, selection: { ...selected, model: 'unsupported' } })).body.result), /preflight failed/);
    assert.equal(journal('source').phase, 'running');
    assert.equal((await request('start', 'target', 0)).body.result, 'not-authority');
    const targetSetupPath = join(dir, 'target', 'setup.json'), savedSetup = readFileSync(targetSetupPath);
    unlinkSync(targetSetupPath);
    assert.match(String((await request('move', 'source', 1, { target: 'target', targetRevision: 0, selection: selected })).body.result), /preflight failed/);
    assert.equal(journal('source').phase, 'running');
    writeFileSync(targetSetupPath, savedSetup, { mode: 0o600 });
    const preservedSource = journal('source');
    for (const mode of ['current-drift', 'config-drift', 'coalesced-drift', 'teardown-drift', 'wrong-protocol', 'missing-profile']) {
      writeFileSync(join(dir, 'mode'), mode);
      assert.match(String((await request('move', 'source', 1, { target: 'target', targetRevision: 0, selection: selected })).body.result), /preflight failed/, mode);
      assert.deepEqual(journal('source').actual, preservedSource.actual, mode);
      assert.deepEqual(journal('source').assignment, preservedSource.assignment, mode);
      assert.equal(journal('source').phase, 'running', mode);
      assert.equal(journal('target').phase, 'stopped', mode);
      for (const pid of sourcePids) assert.doesNotThrow(() => process.kill(pid, 0), mode);
    }
    writeFileSync(join(dir, 'mode'), 'reject');
    assert.match(String((await request('move', 'source', 1, { target: 'target', targetRevision: 0, selection: selected })).body.result), /preflight failed/);
    assert.equal(journal('source').phase, 'running', 'failed identity-free ACP prerequisite probe preserves source');
    assert.equal(journal('target').phase, 'stopped');
    writeFileSync(join(dir, 'mode'), 'ok');


    {
      const targetSocket = [...management.clients].at(-1)!;
      const send = targetSocket.send.bind(targetSocket);
      targetSocket.send = ((data: Parameters<typeof send>[0], ...args: unknown[]) => {
        if (open(JSON.parse(String(data)), ownerSecret).type === 'grant') {
          assert.equal(journal('source').phase, 'stopped');
          assert.equal(journal('source').assignment.assignedHost, 'target');
          for (const pid of sourcePids) assert.throws(() => process.kill(pid, 0), (e: any) => e.code === 'ESRCH', 'source exit before grant delivery / target actual spawn');
          if (outcome === 'reject') writeFileSync(join(dir, 'mode'), 'reject');
        }
        return (send as Function)(data, ...args);
      }) as typeof targetSocket.send;
    }
    expected = second; moved = true;
    assert.equal((await request('move', 'source', 1, { target: 'target', targetRevision: 0, selection: selected })).body.result, 'accepted');
    if (outcome === 'reject') {
      await wait(() => Object.values(journal('target').operations).some((op: any) => String(op.reply.body.result).includes('protocol/model verification failed')));
      assert.equal(journal('target').phase, 'stopped');
      assert.equal(journal('target').actual, null);
      assert.equal(journal('target').assignment.assignedHost, 'target');
      assert.equal(journal('source').phase, 'stopped');
      assert.equal((await request('start', 'source', 2)).body.result, 'not-authority');
      assert.equal(replies.length, 1, 'failed post-grant model acknowledgement cannot publish a reply');
      console.log(`post-grant model rejection: target assigned stopped, source revoked, signer=${agent}`);
      return;
    }
    await wait(() => journal('target').phase === 'running');
    assert.equal(journal('source').phase, 'stopped');
    assert.equal(journal('source').assignment.assignedHost, 'target');
    assert.equal(journal('target').actual.evidence.source, 'external-buzz-conversation');
    assert.equal(journal('target').actual.evidence.model, selected.model);
    assert.equal(journal('target').actual.evidence.agentPublicKey, agent);
    assert.equal(journal('target').actual.evidence.session, 'conversation-session');
    assert.equal(readFileSync(join(dir, 'preflight-no-identity'), 'utf8'), 'yes');
    assert.equal((await request('start', 'source', 2)).body.result, 'not-authority');
    assert.equal(replies.length, 2);
    for (const next of [later]) {
      const previous: number = replies.length; expected = next;
      const c = next.tags.find(t => t[0] === 'h')![1]!;
      const subscription = subscriptionsBySocket.get(c); assert.ok(subscription);
      subscription.socket.send(JSON.stringify(['EVENT', subscription.id, next]));
      for (let i = 0; i < 300 && replies.length === previous; i++) await delay(50);
      assert.equal(replies.length, previous + 1, 'each later conversation must produce its own signed reply');
    }
    for (let i = 0; i < 100 && readFileSync(join(dir, 'completed-tools'), 'utf8').trim().split('\n').length < 3; i++) await delay(50);
    assert.equal(readFileSync(join(dir, 'rejected-mentions'), 'utf8').trim().split('\n').length, 3);
    assert.equal(replies.length, 3); assert.equal(new Set(replies.map(e => e.id)).size, 3);
    assert.ok(channelReads >= 3, 'additional CLI channels list operation exercised');
    assert.equal(readFileSync(join(dir, 'harness-identity'), 'utf8'), agent);
    assert.equal(journal('target').phase, 'running');
    console.log(`isolated installed runtime: NIP-42 verified, subscriptions=${subscriptions}, signed replies=${replies.map(e => e.id).join(',')}, agent=${agent}, two channels/three owner prompts; explicit non-owner member recipient=${recipient}`);

  } finally {
    for (const child of children) { if (child.exitCode === null && child.signalCode === null) { const exit = once(child, 'exit'); child.kill('SIGTERM'); await exit; } }
    ui.close(); for (const socket of management.clients) socket.terminate();
    await new Promise<void>(resolve => management.close(() => resolve()));
    for (const file of ['harness-pid', 'descendant-pid', 'tool-shim-pid']) {
      if (!existsSync(join(dir, file))) continue;
      const pid = Number(readFileSync(join(dir, file), 'utf8'));
      assert.throws(() => process.kill(pid, 0), (e: any) => e.code === 'ESRCH', `${file} must be absent after owned Stop`);
    }
    for (const socket of ws.clients) socket.terminate();
    await new Promise<void>(resolve => ws.close(() => resolve()));
    await new Promise<void>(resolve => http.close(() => resolve()));
    rmSync(dir, { recursive: true, force: true });
  }
});
