import { randomUUID } from 'node:crypto';
import { WebSocketServer, type WebSocket } from 'ws';
import { verifyEvent, type Event } from 'nostr-tools/pure';
import { verifyHostAttestation } from '../src/host-attestation.ts';

/** Narrow executable model of Buzz 051c3a2 admission, NOT a deployed/Rust relay.
 * auth.rs: signed NIP42 + OA ViaOwner membership; api/mod.rs AUTH time semantics.
 * ingest.rs:1059 WebSocket-only, signature, ±900s, 256KB, MessagesWrite,
 * explicit giftwrap author/connection mismatch exception. req.rs p-gate.
 * No DB/token/push implementation; fixtures grant member MessagesWrite only.
 */
export async function nostrFixture(owner: string) {
  const server = new WebSocketServer({ host: '127.0.0.1', port: 0, maxPayload: 300000 });
  await new Promise<void>(resolve => server.once('listening', resolve));
  const address = server.address();
  if (!address || typeof address === 'string') throw Error('Missing fixture address');
  const url = `ws://127.0.0.1:${address.port}`;
  const history: Event[] = [];
  const connections = new Map<WebSocket, { pubkey?: string; subscription?: string }>();
  let independentWraps = 0;
  server.on('connection', socket => {
    const state: { pubkey?: string; subscription?: string } = {};
    connections.set(socket, state);
    const challenge = randomUUID();
    socket.send(JSON.stringify(['AUTH', challenge]));
    socket.on('close', () => connections.delete(socket));
    socket.on('message', bytes => {
      const [type, value, filter] = JSON.parse(bytes.toString());
      const now = Math.floor(Date.now() / 1000);
      if (type === 'AUTH') {
        const event = value as Event;
        try {
          if (!verifyEvent(event) || event.kind !== 22242 || Math.abs(event.created_at - now) > 900 || event.content !== '' || !event.tags.some(t => t[0] === 'relay' && t[1] === url) || !event.tags.some(t => t[0] === 'challenge' && t[1] === challenge)) throw Error('AUTH');
          if (event.pubkey !== owner) verifyHostAttestation(event.tags.find(t => t[0] === 'auth'), event.pubkey, owner, event.created_at);
          state.pubkey = event.pubkey;
          socket.send(JSON.stringify(['OK', event.id, true, 'authenticated']));
        } catch { socket.send(JSON.stringify(['OK', event.id, false, 'restricted: not a relay member'])); }
      } else if (type === 'REQ') {
        if (!state.pubkey || JSON.stringify(filter) !== JSON.stringify({ kinds: [1059], '#p': [state.pubkey] })) {
          socket.send(JSON.stringify(['CLOSED', value, 'restricted: p gate'])); return;
        }
        state.subscription = value;
        for (const event of history) if (event.tags.some(t => t[0] === 'p' && t[1] === state.pubkey)) socket.send(JSON.stringify(['EVENT', value, event]));
        socket.send(JSON.stringify(['EOSE', value]));
      } else if (type === 'EVENT') {
        const event = value as Event;
        const accepted = Boolean(state.pubkey && verifyEvent(event) && event.kind === 1059 && Math.abs(event.created_at - now) <= 900 && Buffer.byteLength(event.content) <= 262144);
        socket.send(JSON.stringify(['OK', event.id, accepted, accepted ? 'accepted' : 'invalid']));
        if (!accepted) return;
        if (event.pubkey !== state.pubkey) independentWraps++;
        if (history.some(e => e.id === event.id)) return;
        history.push(event);
        for (const [peer, target] of connections) if (target.subscription && event.tags.some(t => t[0] === 'p' && t[1] === target.pubkey)) peer.send(JSON.stringify(['EVENT', target.subscription, event]));
      }
    });
  });
  return { url, history, get independentWraps() { return independentWraps; }, async close() { for (const peer of connections.keys()) peer.terminate(); await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve())); } };
}
