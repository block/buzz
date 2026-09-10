import { randomUUID } from 'node:crypto';
import { WebSocket } from 'ws';
import { finalizeEvent, getPublicKey, type Event } from 'nostr-tools/pure';
import { unwrapManagement, wrapManagement, type AuthenticatedMessage } from './nostr-codec.ts';
import type { Message } from './protocol.ts';

/** One authenticated subscription generation. Caller owns durable retries and operation
 * completion; accepted publication is ONLY a relay ACK. No implicit effect retries.
 * A new instance resubscribes full history, deliberately overlapping live delivery.
 */
export function connectNostr(url: string, secret: Uint8Array, receive: (value: AuthenticatedMessage) => void, disconnected: (error: Error) => void) {
  const endpoint = new URL(url);
  if (endpoint.username || endpoint.password || endpoint.hash || !(endpoint.protocol === 'wss:' || (endpoint.protocol === 'ws:' && endpoint.hostname === '127.0.0.1'))) throw Error('Nostr transport requires wss (loopback ws fixtures only)');
  const socket = new WebSocket(url, { maxPayload: 300000, handshakeTimeout: 2000 });
  const subscription = randomUUID();
  let ended = false; let subscribed = false; let requested = false; let authentication: string | undefined;
  let challenges = 0;
  let resolveReady!: () => void; let rejectReady!: (error: Error) => void;
  const ready = new Promise<void>((resolve, reject) => { resolveReady = resolve; rejectReady = reject; });
  const pending = new Map<string, { resolve: () => void; reject: (error: Error) => void; timer: ReturnType<typeof setTimeout> }>();
  let startup = setTimeout(() => fail(Error('Nostr authentication/subscription timeout')), 5000);
  function fail(error: Error) {
    if (ended) return;
    ended = true; subscribed = false; clearTimeout(startup); rejectReady(error);
    for (const item of pending.values()) { clearTimeout(item.timer); item.reject(error); }
    pending.clear(); socket.close(); disconnected(error);
  }
  function send(frame: unknown[]) {
    if (ended || socket.readyState !== WebSocket.OPEN || socket.bufferedAmount > 1048576) throw Error('Nostr disconnected/backpressured; result unknown');
    socket.send(JSON.stringify(frame), error => { if (error) fail(error); });
  }
  socket.on('error', fail);
  socket.on('close', () => fail(Error('Nostr closed; unreceipted operations remain unknown')));
  socket.on('message', bytes => {
    if (ended) return;
    let frame: unknown;
    try { frame = JSON.parse(bytes.toString()); } catch { fail(Error('Malformed relay frame')); return; }
    if (!Array.isArray(frame)) { fail(Error('Malformed relay frame')); return; }
    const [type, id, payload] = frame;
    if (type === 'AUTH') {
      if (typeof id !== 'string' || id.length > 1024 || ++challenges > 4) { fail(Error('Invalid/excessive AUTH challenges')); return; }
      subscribed = false; requested = false;
      clearTimeout(startup);
      startup = setTimeout(() => fail(Error('Nostr authentication/subscription timeout')), 5000);
      const event = finalizeEvent({ kind: 22242, created_at: Math.floor(Date.now() / 1000), content: '', tags: [['relay', url], ['challenge', id]] }, secret);
      authentication = event.id;
      try { send(['AUTH', event]); } catch (error) { fail(error as Error); }
    } else if (type === 'OK' && typeof id === 'string') {
      if (id === authentication) {
        if (payload !== true) { fail(Error('Relay authentication denied')); return; }
        authentication = undefined;
        try { requested = true; send(['REQ', subscription, { kinds: [1059], '#p': [getPublicKey(secret)] }]); } catch (error) { fail(error as Error); }
      } else {
        const item = pending.get(id);
        if (item) { clearTimeout(item.timer); pending.delete(id); payload === true ? item.resolve() : item.reject(Error('Relay rejected publication')); }
      }
    } else if (type === 'EOSE' && id === subscription && requested) {
      subscribed = true; clearTimeout(startup); resolveReady();
    } else if (type === 'CLOSED' && id === subscription) {
      fail(Error('Relay closed subscription'));
    } else if (type === 'EVENT' && id === subscription && requested) {
      let decoded: AuthenticatedMessage;
      try { decoded = unwrapManagement(payload, secret); } catch { return; }
      // Persistence/authority failures are application failures, not malformed ciphertext.
      receive(decoded);
    }
  });
  return {
    ready,
    get socket() { return socket; },
    /** Resolves on relay acceptance, never on host operation completion. */
    publish(message: Message, recipient: string): Promise<void> {
      if (!subscribed || ended) return Promise.reject(Error('Nostr not subscribed; result unknown'));
      if (pending.size >= 64) return Promise.reject(Error('Nostr publication backlog full'));
      let event: Event;
      try { event = wrapManagement(message, secret, recipient); } catch (error) { return Promise.reject(error); }
      return new Promise<void>((resolve, reject) => {
        const timer = setTimeout(() => { pending.delete(event.id); reject(Error('Relay ACK timeout; result unknown')); }, 5000);
        pending.set(event.id, { resolve, reject, timer });
        try { send(['EVENT', event]); } catch (error) { clearTimeout(timer); pending.delete(event.id); reject(error); }
      });
    },
    close() { fail(Error('Nostr client closed')); },
  };
}
