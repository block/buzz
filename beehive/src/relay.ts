import { trace } from './latency-trace.ts';
import { WebSocketServer, WebSocket } from 'ws';
import { existsSync } from 'node:fs';
import { verifyEnvelope, type Envelope } from './protocol.ts';
import { readPrivate } from './storage.ts';
import { writeRelaySnapshot, UncertainSnapshot, type SnapshotBoundary } from './relay-storage.ts';

/** Isolated dev relay: loopback-only, one admitted owner, bounded durable ciphertext log. */
export async function relay(port: number, owner: string, path: string, boundary?: SnapshotBoundary) {
  if (!/^[0-9a-f]{64}$/.test(owner)) throw Error('Invalid owner public key');
  const history: Envelope[] = existsSync(path) ? (readPrivate(path) as unknown[]).map(e => verifyEnvelope(e,owner)) : [];
  const known = new Set(history.map(e => e.signature));
  const server = new WebSocketServer({ host: '127.0.0.1', port, maxPayload: 70000 });
  let queue = Promise.resolve();
  let closing = false;
  let fatal: Error | undefined;
  let pending = 0;
  let active = false;
  const waiting: { e: Envelope; socket: WebSocket }[] = [];
  const close = server.close.bind(server);
  // Stop admission immediately; accepted queue entries drain before close completes.
  server.close = callback => {
    closing = true;
    void queue.then(() => close(error => callback?.(fatal ?? error)));
  };
  async function commit() {
    while (waiting.length) {
      const batch = waiting.splice(0); trace('relay.batch', batch.map(x => x.e.signature));
      const additions: Envelope[] = [];
      const signatures = new Set<string>();
      for (const { e, socket } of batch) {
        if (fatal) { socket.close(1011, 'Relay storage uncertain'); continue; }
        if (known.has(e.signature) || signatures.has(e.signature)) continue;
        if (history.length + additions.length >= 10000) {
          socket.close(1008, 'Development relay log full'); continue;
        }
        additions.push(e); signatures.add(e.signature);
      }
      if (additions.length) {
        try {
          // One atomic snapshot for this received prefix, never concurrent writers.
          await writeRelaySnapshot(path, [...history, ...additions], boundary);
        } catch (error) {
          // A failed batch is explicitly unavailable; a later batch can still
          // commit after pre-replacement failure. Replacement uncertainty fences all.
          if (error instanceof UncertainSnapshot) {
            fatal = error;
            for (const client of server.clients) client.close(1011, 'Relay storage uncertain');
          } else for (const { socket } of batch) socket.close(1011, 'Publication storage failed');
          pending -= batch.length;
          continue;
        }
        trace('relay.durable', additions.map(e => e.signature)); history.push(...additions);
        for (const e of additions) {
          known.add(e.signature);
          for (const client of server.clients) if (client.readyState === WebSocket.OPEN) {
            if (client.bufferedAmount > 1000000) client.close(1008, 'Slow reader');
            else { trace('relay.broadcast', { signature: e.signature }); client.send(JSON.stringify(e)); }
          }
        }

      }
      pending -= batch.length;
    }
  }
  server.on('connection', socket => {
    socket.on('error', () => {});
    if (closing || fatal) { socket.close(1011, 'Relay unavailable'); return; }
    // Ciphertext only. Authenticated writes; unauthenticated local readers learn traffic metadata.
    for (const e of history) socket.send(JSON.stringify(e));
    socket.on('message', data => {
      if (closing || fatal) { socket.close(1011, 'Relay unavailable'); return; }
      let e: Envelope;
      try { e = verifyEnvelope(JSON.parse(data.toString()), owner); }
      catch { socket.close(1008, 'Invalid publication'); return; }
      if (pending >= 10000) { socket.close(1013, 'Publication queue full'); return; }
      trace('relay.enqueue', { signature: e.signature, pending }); pending++;
      waiting.push({ e, socket });
      if (active) return;
      active = true;
      queue = Promise.resolve().then(commit).catch(error => {
        fatal = error instanceof Error ? error : Error(String(error));
        waiting.length = 0; pending = 0;
        for (const client of server.clients) client.close(1011, 'Relay unavailable');
      }).finally(() => { active = false; });
    });
  });
  await new Promise<void>((resolve,reject) => { server.once('listening',resolve); server.once('error',reject); });
  return server;
}
