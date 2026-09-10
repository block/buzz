import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { WebSocketServer } from 'ws';
import { mkdtempSync, realpathSync, writeFileSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createHash } from 'node:crypto';
import { schnorr } from '@noble/curves/secp256k1';
import { setTimeout as delay } from 'node:timers/promises';
import { ConversationSession } from '../src/broker.ts';
import { newKey, publicKey } from '../src/protocol.ts';

// Explicitly opt-in installed executable; never opens an owner profile or provider.
test('installed buzz-acp authenticates and completes the same identity-bearing conversation through broker', { skip: !process.env.BEEHIVE_REAL_BUZZ_ACP }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-installed-')));
  writeFileSync(join(dir, 'mode'), 'ok');
  const secret = newKey(); const ownerSecret = newKey(); const agent = publicKey(secret); const owner = publicKey(ownerSecret);
  const channel = 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee';
  let authenticated = false; let subscriptions = 0; let delivered = false;
  function observeReply(e: any) {
    if (![9, 40002].includes(e.kind) || e.pubkey !== agent) return;
    assert.ok(schnorr.verify(e.sig, e.id, agent));
    assert.ok(e.tags.some((t: string[]) => t[0] === 'h' && t[1] === channel));
    assert.ok(e.tags.some((t: string[]) => t[0] === 'e' && t[1] === inbound.id));
    assert.ok(e.content.includes('Private conversation fixture response'));
    delivered = true;
  }
  function event(kind: number, tags: string[][], content: string, key = ownerSecret) {
    const pubkey = publicKey(key); const created_at = Math.floor(Date.now() / 1000);
    const id = createHash('sha256').update(JSON.stringify([0, pubkey, created_at, kind, tags, content])).digest('hex');
    return { id, pubkey, created_at, kind, tags, content, sig: Buffer.from(schnorr.sign(id, key)).toString('hex') };
  }
  const members = event(39002, [['d', channel], ['p', agent], ['p', owner]], '');
  const metadata = event(39000, [['d', channel], ['name', 'fixture'], ['type', 'channel']], '');
  const inbound = event(9, [['h', channel], ['p', agent]], 'Please respond to this isolated fixture conversation.');
  const http = createServer(async (req, res) => {
    let body = ''; for await (const chunk of req) body += chunk;
    res.setHeader('Content-Type', 'application/json');
    if (req.url?.includes('/query')) {
      const filters = JSON.parse(body); const serialized = JSON.stringify(filters);
      res.end(JSON.stringify(serialized.includes('39002') ? [members] : serialized.includes('39000') ? [metadata] : []));
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
        if (m.slice(2).some((f: any) => f['#h']?.includes(channel))) setTimeout(() => socket.send(JSON.stringify(['EVENT', m[1], inbound])), 100);
      }
    });
  });
  await new Promise<void>(resolve => http.listen(0, '127.0.0.1', resolve));
  const address = http.address(); assert.ok(address && typeof address !== 'string');
  const session = new ConversationSession({ executable: realpathSync(process.env.BEEHIVE_REAL_BUZZ_ACP!), relay: `ws://127.0.0.1:${address.port}` }, {
    executable: realpathSync(process.execPath), args: [resolve('test/conversation-harness-fixture.ts')], workspace: dir, home: dir, configDirectory: dir,
    databricksHost: 'https://fixture.invalid', model: 'databricks-claude-haiku-4-5',
  }, secret, owner, 10_000);
  try {
    await session.ready;
    for (let i = 0; i < 100 && !subscriptions; i++) await delay(50);
    assert.ok(authenticated, 'installed executable must authenticate with the provisioned key');
    assert.ok(subscriptions > 0, 'installed executable must enter subscription loop');
    const evidence = await session.verify();
    assert.equal(evidence.session, 'conversation-session');
    console.log(`isolated installed runtime: NIP-42 verified, subscriptions=${subscriptions}, harnessSpawned=${existsSync(join(dir, 'harness-pid'))}, same-session completed=true, signed threaded reply observed=${delivered}`);
  } finally {
    await session.stop(); for (const socket of ws.clients) socket.terminate();
    await new Promise<void>(resolve => ws.close(() => resolve()));
    await new Promise<void>(resolve => http.close(() => resolve()));
    rmSync(dir, { recursive: true, force: true });
  }
});
