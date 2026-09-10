import { WebSocket } from 'ws';
import { open, seal, type Message } from './protocol.ts';
/** Validate transport before acquiring installation resources. */
export function validateRelayURL(url: string) {
  const endpoint = new URL(url);
  if (endpoint.protocol !== 'ws:' || endpoint.hostname !== '127.0.0.1') throw Error('Development transport requires ws://127.0.0.1');
}
/** All UI/host communication uses the real relay, never a host API. */
export function connect(url: string, secret: string, receive: (m: Message) => void) {
  validateRelayURL(url);
  const socket = new WebSocket(url, { maxPayload: 70000 });
  socket.on('message', data => {
    try { receive(open(JSON.parse(data.toString()),secret)); } catch { /* Reject malformed/untrusted data, never print payloads. */ }
  });
  socket.on('error', () => {});
  return {
    socket,
    ready: new Promise<void>((resolve,reject) => { socket.once('open',resolve); socket.once('error',reject); }),
    send(m: Message) { if (socket.readyState !== WebSocket.OPEN) throw Error('Relay disconnected; result unknown'); socket.send(JSON.stringify(seal(m,secret))); },
    close() { socket.close(); },
  };
}
