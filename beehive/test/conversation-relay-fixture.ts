import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { WebSocketServer } from 'ws';
import { finalizeEvent, verifyEvent, type Event } from 'nostr-tools/pure';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { newKey, publicKey } from '../src/protocol.ts';

/** Isolated conversation protocol fixture, not production relay admission evidence. */
export async function conversationRelayFixture(directory: string, ownerSecret: string, agent: string) {
  const owner = publicKey(ownerSecret), recipient = publicKey(newKey());
  const channel = 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee';
  writeFileSync(join(directory, 'mode'), 'ok');
  writeFileSync(join(directory, 'recipient'), recipient);
  writeFileSync(join(directory, 'outsider'), publicKey(newKey()));
  const event = (kind: number, tags: string[][], content: string) => finalizeEvent({ kind, tags, content, created_at: Math.floor(Date.now() / 1000) }, Buffer.from(ownerSecret, 'hex'));
  const members = event(39002, [['d', channel], ['p', agent], ['p', owner], ['p', recipient]], '');
  const metadata = event(39000, [['d', channel], ['name', 'fixture'], ['type', 'channel']], '');
  const inbound = event(9, [['h', channel], ['p', agent]], 'Please respond to this isolated fixture conversation.');
  const replies: Event[] = [];
  function observe(value: Event) {
    if (![9, 40002].includes(value.kind)) return;
    assert.equal(value.pubkey, agent);
    assert.ok(verifyEvent(value));
    assert.deepEqual(value.tags.filter(t => t[0] === 'p').map(t => t[1]), [recipient]);
    assert.ok(value.tags.some(t => t[0] === 'h' && t[1] === channel));
    assert.ok(value.tags.some(t => t[0] === 'e' && t[1] === inbound.id));
    assert.match(value.content, /Private conversation fixture response/);
    replies.push(value);
  }
  const http = createServer(async (req, res) => {
    let body = ''; for await (const chunk of req) body += chunk;
    res.setHeader('Content-Type', 'application/json');
    if (req.url?.includes('/query')) {
      const query = JSON.stringify(JSON.parse(body));
      res.end(JSON.stringify(query.includes('39002') ? [members] : query.includes('39000') ? [metadata] : query.includes(inbound.id) ? [inbound] : []));
    } else if (req.url?.includes('/events')) {
      const value = JSON.parse(body); observe(value); res.end(JSON.stringify({ id: value.id, accepted: true }));
    } else res.end('[]');
  });
  const ws = new WebSocketServer({ server: http });
  ws.on('connection', socket => {
    let authenticated = false;
    socket.send(JSON.stringify(['AUTH', 'isolated-conversation-challenge']));
    socket.on('message', bytes => {
      const [type, value, ...filters] = JSON.parse(bytes.toString());
      if (type === 'AUTH') {
        assert.ok(verifyEvent(value)); assert.equal(value.pubkey, agent); assert.equal(value.kind, 22242);
        assert.ok(value.tags.some((t: string[]) => t[0] === 'challenge' && t[1] === 'isolated-conversation-challenge'));
        assert.ok(!value.tags.some((t: string[]) => t[0] === 'auth'));
        authenticated = true; socket.send(JSON.stringify(['OK', value.id, true, '']));
      } else if (type === 'REQ') {
        assert.ok(authenticated);
        socket.send(JSON.stringify(['EOSE', value]));
        if (filters.some((f: any) => f['#h']?.includes(channel))) {
          setTimeout(() => { if (socket.readyState === 1) socket.send(JSON.stringify(['EVENT', value, inbound])); }, 100);
        }
      } else if (type === 'EVENT') {
        assert.ok(authenticated); observe(value); socket.send(JSON.stringify(['OK', value.id, true, '']));
      }
    });
  });
  await new Promise<void>(resolve => http.listen(0, '127.0.0.1', resolve));
  const address = http.address(); assert.ok(address && typeof address !== 'string');
  return { replies, url: `ws://127.0.0.1:${address.port}`, async close() {
    for (const socket of ws.clients) socket.terminate();
    await new Promise<void>((resolve, reject) => ws.close(e => e ? reject(e) : resolve()));
    await new Promise<void>((resolve, reject) => http.close(e => e ? reject(e) : resolve()));
  } };
}
