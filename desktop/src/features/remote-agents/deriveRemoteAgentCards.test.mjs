import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  deriveHealthFromStatus,
  deriveRemoteAgentCards,
} from "./deriveRemoteAgentCards.ts";

describe("deriveRemoteAgentCards", () => {
  it("marks unreachable when fetch failed", () => {
    const h = deriveHealthFromStatus(null, true, 1_000_000);
    assert.equal(h.health, "unknown");
    assert.match(h.label, /unreachable/);
  });

  it("maps seats to cards with host metadata", () => {
    const now = Math.floor(Date.now() / 1000);
    const cards = deriveRemoteAgentCards(
      {
        ok: true,
        host_id: "asus-g501vw",
        host_role: "home",
        ts: now,
        relay: { ok: true },
        ollama: { ok: true, models: ["gemma3:4b"] },
        watchers: { process_matches: 2, unit_pids: 1 },
        seats: [
          {
            seat_id: "home-grok",
            model: "gemma3:4b",
            runtimes: ["watch", "local-llm"],
            expected_online: true,
            channels: ["92297894-c2e8-4df1-a710-d1cfd1032d5e"],
          },
        ],
      },
      false,
    );
    assert.equal(cards.length, 1);
    assert.equal(cards[0].seatId, "home-grok");
    assert.equal(cards[0].hostId, "asus-g501vw");
    assert.equal(cards[0].health, "online");
  });
});
