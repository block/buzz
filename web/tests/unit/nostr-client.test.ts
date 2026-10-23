import assert from "node:assert/strict";
import { test } from "node:test";

import {
  NostrClosedBeforeEoseError,
  queryEvents,
} from "../../src/shared/lib/nostr-client.ts";

class FakeWebSocket extends EventTarget {
  static last: FakeWebSocket | undefined;
  readonly sent: string[] = [];
  readonly url: string;

  constructor(url: string) {
    super();
    this.url = url;
    FakeWebSocket.last = this;
  }

  send(data: string): void {
    this.sent.push(data);
  }

  close(): void {}
}

test("queryEvents rejects when the socket closes before EOSE", async () => {
  const realWebSocket = globalThis.WebSocket;
  Object.defineProperty(globalThis, "WebSocket", {
    configurable: true,
    value: FakeWebSocket,
  });

  try {
    const query = queryEvents("ws://relay.test", { kinds: [1] });
    const socket = FakeWebSocket.last;
    assert.ok(socket, "queryEvents constructed a WebSocket");

    socket.dispatchEvent(new Event("open"));
    socket.dispatchEvent(new Event("close"));

    await assert.rejects(query, NostrClosedBeforeEoseError);
  } finally {
    Object.defineProperty(globalThis, "WebSocket", {
      configurable: true,
      value: realWebSocket,
    });
    FakeWebSocket.last = undefined;
  }
});
