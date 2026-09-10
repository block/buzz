import { createServer } from 'node:http';
import { randomUUID } from 'node:crypto';
import { WebSocketServer, type WebSocket } from 'ws';
import { verifyEvent, type Event } from 'nostr-tools/pure';

/** Private NIP42/59 relay fixture, NOT a deployed/Rust relay. Enforces signatures,
 * recipient-scoped REQ, timestamp/size and publication rules. Optional allowed
 * identities model real server AUTH refusal, not a Beehive membership preflight.
 */
export async function nostrFixture(owner: string, allowedIdentities?: ReadonlySet<string>, access: { open?: boolean; denyAuth?: boolean; denyPublication?: boolean; denyReq?: boolean } = {}) {
  const allowed = new Set(allowedIdentities ?? [owner]);
  const httpRequests: string[] = [];
  const http = createServer((req, res) => {
    httpRequests.push(`${req.method} ${req.url}`);
    res.setHeader('Content-Type', 'application/json');
    if (req.method === 'GET' && req.url === '/') res.end(JSON.stringify({ supported_nips: [1,2,10,11,16,17,23,25,29,33,38,42,50,56] }));
    else res.writeHead(404).end('{}');
  });
  const server = new WebSocketServer({ server: http, maxPayload: 300000 });
  await new Promise<void>(resolve => http.listen(0, '127.0.0.1', resolve));
  const address = http.address();
  if (!address || typeof address === 'string') throw Error('Missing fixture address');
  const url = `ws://127.0.0.1:${address.port}`;
  const history: Event[] = [];
  const connections = new Map<WebSocket, { pubkey?: string; subscription?: string }>();
  let independentWraps = 0, connectionsOpened = 0;
  server.on('connection', socket => {
    connectionsOpened++;
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
          if (!verifyEvent(event) || event.kind !== 22242 || Math.abs(event.created_at - now) > 60 || event.content !== '' || !event.tags.some(t => t[0] === 'relay' && t[1] === url) || !event.tags.some(t => t[0] === 'challenge' && t[1] === challenge)) throw Error('AUTH');
          if (access.denyAuth) throw Error('fixture AUTH refusal');
          if (event.tags.some(t => t[0] === 'auth')) throw Error('OA prohibited');
          if (!access.open && !allowed.has(event.pubkey)) throw Error('AUTH refused');
          state.pubkey = event.pubkey;
          socket.send(JSON.stringify(['OK', event.id, true, 'authenticated']));
        } catch { socket.send(JSON.stringify(['OK', event.id, false, 'restricted: not a relay member'])); }
      } else if (type === 'REQ') {
        if (access.denyReq || !state.pubkey || JSON.stringify(filter) !== JSON.stringify({ kinds: [1059], '#p': [state.pubkey] })) {
          socket.send(JSON.stringify(['CLOSED', value, 'restricted: p gate'])); return;
        }
        state.subscription = value;
        for (const event of history) if (event.tags.some(t => t[0] === 'p' && t[1] === state.pubkey)) socket.send(JSON.stringify(['EVENT', value, event]));
        socket.send(JSON.stringify(['EOSE', value]));
      } else if (type === 'EVENT') {
        const event = value as Event;
        const accepted = Boolean(!access.denyPublication && state.pubkey && verifyEvent(event) && event.kind === 1059 && Math.abs(event.created_at - now) <= 900 && Buffer.byteLength(event.content) <= 262144);
        socket.send(JSON.stringify(['OK', event.id, accepted, accepted ? 'accepted' : 'invalid']));
        if (!accepted) return;
        if (event.pubkey !== state.pubkey) independentWraps++;
        if (history.some(e => e.id === event.id)) return;
        history.push(event);
        for (const [peer, target] of connections) if (target.subscription && event.tags.some(t => t[0] === 'p' && t[1] === target.pubkey)) peer.send(JSON.stringify(['EVENT', target.subscription, event]));
      }
    });
  });
  return { url, history, httpRequests, get connectionsOpened() { return connectionsOpened; }, get independentWraps() { return independentWraps; }, async close() { for (const peer of connections.keys()) peer.terminate(); await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve())); await new Promise<void>((resolve, reject) => http.close(error => error ? reject(error) : resolve())); } };
}
