// Browser implementation of tauri-plugin-websocket as used by relayClientSession.
type Sink = { onmessage: (m: unknown) => void };
const sockets = new Map<number, WebSocket>();
let next = 1;

export function wsConnect(url: string, sink: Sink): Promise<number> {
  return new Promise((resolve, reject) => {
    const id = next++;
    const ws = new WebSocket(url);
    sockets.set(id, ws);
    ws.onopen = () => resolve(id);
    ws.onerror = () => {
      reject(new Error("WebSocket connection failed"));
      sink.onmessage({ type: "Close", data: { code: 1006, reason: "error" } });
    };
    ws.onmessage = (e) =>
      sink.onmessage({
        type: "Text",
        data: typeof e.data === "string" ? e.data : "",
      });
    ws.onclose = (e) => {
      sockets.delete(id);
      sink.onmessage({
        type: "Close",
        data: { code: e.code, reason: e.reason },
      });
    };
  });
}
export function wsSend(id: number, message: { type: string; data: string }) {
  const ws = sockets.get(id);
  if (!ws) throw new Error(`WebSocket ${id} not found`);
  ws.send(message.data);
}
export function wsClose(id: number) {
  sockets.get(id)?.close();
  sockets.delete(id);
}
export function wsCloseAll() {
  for (const id of [...sockets.keys()]) wsClose(id);
}
