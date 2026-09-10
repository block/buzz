import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { WebSocketServer } from 'ws';
import { mkdtempSync, realpathSync, writeFileSync, rmSync, existsSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createHash } from 'node:crypto';
import { schnorr } from '@noble/curves/secp256k1';
import { setTimeout as delay } from 'node:timers/promises';
import { ConversationSession } from '../src/broker.ts';
import { newKey, publicKey } from '../src/protocol.ts';

// Explicitly opt-in installed executable; never opens an owner profile or provider.
test('installed buzz-acp completes multi-conversation signed replies and normal CLI tools through the actual Buzz CLI', { skip: !process.env.BEEHIVE_REAL_BUZZ_ACP }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-installed-')));
  writeFileSync(join(dir, 'mode'), 'ok');
  const secret = newKey(); const ownerSecret = newKey(); const agent = publicKey(secret); const owner = publicKey(ownerSecret);
  const channel = 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee';
  const replies: any[] = [];
  const subscriptionsBySocket = new Map<string, { socket: any; id: string }>();
  let expected: any;
  let channelReads = 0;
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
        authenticated = true; socket.send(JSON.stringify(['OK', e.id, true, '']));
      } else if (m[0] === 'EVENT') { observeReply(m[1]); socket.send(JSON.stringify(['OK', m[1].id, true, ''])); }
      else if (m[0] === 'REQ') {
        subscriptions++; socket.send(JSON.stringify(['EOSE', m[1]]));
        for (const f of m.slice(2)) for (const c of f['#h'] ?? []) { subscriptionsBySocket.set(c, { socket, id: m[1] }); }
        if (m.slice(2).some((f: any) => f['#h']?.includes(channel))) setTimeout(() => socket.send(JSON.stringify(['EVENT', m[1], inbound])), 100);
      }
    });
  });
  await new Promise<void>(resolve => http.listen(0, '127.0.0.1', resolve));
  const address = http.address(); assert.ok(address && typeof address !== 'string');
  const session = new ConversationSession({ executable: realpathSync(process.env.BEEHIVE_REAL_BUZZ_ACP!), relay: `ws://127.0.0.1:${address.port}`, replyTool: { executable: realpathSync(join(process.env.BEEHIVE_REAL_BUZZ_ACP!, '..', 'buzz')) } }, {
    executable: realpathSync(process.execPath), args: [resolve('test/conversation-harness-fixture.ts')], workspace: dir, home: dir, configDirectory: dir,
    instructions: 'BEHAVIOR-PROFILE-BOUNDARY: explain assumptions before acting.',
    databricksHost: 'https://fixture.invalid', model: 'databricks-claude-haiku-4-5',
  }, secret, owner, 20_000);
  try {
    await session.ready;
    for (let i = 0; i < 100 && !subscriptions; i++) await delay(50);
    assert.ok(authenticated, 'installed executable must authenticate with the provisioned key');
    assert.ok(subscriptions > 0, 'installed executable must enter subscription loop');
    const evidence = await session.verify();
    assert.equal(readFileSync(join(dir, 'received-system-instructions'), 'utf8'), 'BEHAVIOR-PROFILE-BOUNDARY: explain assumptions before acting.');
    assert.match(readFileSync(join(dir, 'received-prompts.jsonl'), 'utf8'), /BEHAVIOR-PROFILE-BOUNDARY/);
    assert.equal(evidence.session, 'conversation-session');
    assert.equal(evidence.agentPublicKey, agent);
    assert.ok(delivered, 'the actual Buzz CLI must publish a signed threaded agent reply, not just return tool JSON');
    const unauthorized = event(9, [['h', channel], ['p', agent]], 'Not the configured owner.', recipientKey);
    const firstSubscription = subscriptionsBySocket.get(channel)!;
    firstSubscription.socket.send(JSON.stringify(['EVENT', firstSubscription.id, unauthorized]));
    await delay(1200); assert.equal(replies.length, 1, 'upstream owner-only admission rejects a different sender');
    for (const next of [second, later]) {
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
    await delay(100); assert.ok(session.healthy, 'broker remains model-confirmed and healthy after later turns');
    console.log(`isolated installed runtime: NIP-42 verified, subscriptions=${subscriptions}, signed replies=${replies.map(e => e.id).join(',')}, agent=${agent}, two channels/three owner prompts; explicit non-owner member recipient=${recipient}`);

  } finally {
    await session.stop();
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
