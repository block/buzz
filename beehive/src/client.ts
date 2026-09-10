import { WebSocket } from 'ws';
import { open, seal, type Message } from './protocol.ts';
/** Validate transport before acquiring installation resources. */
export function validateRelayURL(url: string) {
  const endpoint = new URL(url);
  if (endpoint.protocol !== 'ws:' || endpoint.hostname !== '127.0.0.1') throw Error('Development transport requires ws://127.0.0.1');
}
/** All UI/host communication uses the real relay, never a host API.
 * Opt-in recovery is bounded to eight reconnect attempts per connection lifetime.
 * Initial connection failure still rejects ready, allowing installation lock unwind.
 * The caller owns durable intent/receipts; an open socket is not an operation ACK.
 */
export function connect(url: string, secret: string, receive: (m: Message) => void, recovered?: () => void) {
  validateRelayURL(url);
  let closed = false; let established = false; let attempts = 0;
  let retry: ReturnType<typeof setTimeout> | undefined;
  let resolveReady: () => void; let rejectReady: (error: Error) => void;
  const ready = new Promise<void>((resolve, reject) => { resolveReady = resolve; rejectReady = reject; });
  function dial(): WebSocket {
    const current = new WebSocket(url, { maxPayload: 70000, handshakeTimeout: 2000 });
    current.on('open', () => {
      if (closed || current !== socket) { current.close(); return; }
      if (!established) { established = true; resolveReady(); }
      else recovered?.();
    });
    current.on('message', data => {
      if (closed || current !== socket) return;
      try { receive(open(JSON.parse(data.toString()),secret)); } catch { /* Reject malformed/untrusted data, never print payloads. */ }
    });
    current.on('error', error => { if (!established) rejectReady(error); });
    current.on('close', () => {
      if (!established) rejectReady(Error('Relay closed before ready'));
      if (closed || current !== socket || !established || !recovered || attempts >= 8) return;
      retry = setTimeout(() => { retry = undefined; if (!closed) socket = dial(); }, Math.min(2000, 100 * 2 ** attempts++));
    });
    return current;
  }
  let socket = dial();
  return {
    get socket() { return socket; },
    ready,
    send(m: Message) { if (closed || socket.readyState !== WebSocket.OPEN) throw Error('Relay disconnected; result unknown'); socket.send(JSON.stringify(seal(m,secret))); },
    close() { closed = true; clearTimeout(retry); socket.close(); },
  };
}
