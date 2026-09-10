import { WebSocketServer, WebSocket } from 'ws';
import { existsSync } from 'node:fs';
import { verifyEnvelope, type Envelope } from './protocol.ts';
import { readPrivate, writePrivate } from './storage.ts';

/** Isolated dev relay: loopback-only, one admitted owner, bounded durable ciphertext log. */
export async function relay(port: number, owner: string, path: string) {
  if (!/^[0-9a-f]{64}$/.test(owner)) throw Error('Invalid owner public key');
  const history: Envelope[] = existsSync(path) ? (readPrivate(path) as unknown[]).map(e => verifyEnvelope(e,owner)) : [];
  const known = new Set(history.map(e => e.signature));
  const server = new WebSocketServer({ host: '127.0.0.1', port, maxPayload: 70000 });
  server.on('connection', socket => {
    // Ciphertext only. Authenticated writes; unauthenticated local readers learn traffic metadata.
    for (const e of history) socket.send(JSON.stringify(e));
    socket.on('error', () => {});
    socket.on('message', data => {
      try {
        const e = verifyEnvelope(JSON.parse(data.toString()),owner);
        if (known.has(e.signature)) return;
        if (history.length >= 10000) throw Error('Development relay log full');
        writePrivate(path,[...history,e]);
        history.push(e); known.add(e.signature);
        for (const client of server.clients) if (client.readyState === WebSocket.OPEN) {
          if (client.bufferedAmount > 1000000) client.close(1008,'Slow reader');
          else client.send(JSON.stringify(e));
        }
      } catch { socket.close(1008,'Invalid or unavailable publication'); }
    });
  });
  await new Promise<void>((resolve,reject) => { server.once('listening',resolve); server.once('error',reject); });
  return server;
}
