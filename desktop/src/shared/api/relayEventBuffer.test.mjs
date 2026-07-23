import assert from "node:assert/strict";
import test from "node:test";

import { deliverBufferedSubscriptionEvents } from "./relayEventBuffer.ts";

test("event-buffer drain delivers only still-live subscriptions in order", () => {
  const delivered = [];
  const subscriptions = new Map([
    [
      "live",
      {
        mode: "live",
        filter: { kinds: [9], limit: 50 },
        onEvent: (event) => delivered.push(event.id),
      },
    ],
    [
      "history",
      {
        mode: "history",
        events: [],
        resolve: () => {},
        reject: () => {},
        timeout: 1,
      },
    ],
  ]);

  deliverBufferedSubscriptionEvents(
    [
      { subId: "removed", event: { id: "removed" } },
      { subId: "live", event: { id: "first" } },
      { subId: "history", event: { id: "history" } },
      { subId: "live", event: { id: "second" } },
    ],
    subscriptions,
  );

  assert.deepEqual(delivered, ["first", "second"]);
});
