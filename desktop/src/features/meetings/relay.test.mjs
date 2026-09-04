import assert from "node:assert/strict";
import test from "node:test";

import {
  getMeetingToken,
  getPaymentStatus,
  listRooms,
  listRoomsByPubkey,
  moderateRoom,
  registerRoom,
  subscribe,
} from "./relay.ts";
import { participantActionPayload } from "./moderationPayloads.ts";

const RELAY_WS = "wss://relay.example";
const API_BASE = "https://l402relay.exe.xyz";
const CAP = { apiBase: API_BASE };

/**
 * Stub `window.__TAURI_INTERNALS__.invoke` so `signRelayEvent` works in node.
 * The stubbed signer echoes the input tags back inside the event so tests can
 * decode the `Authorization` / `X-Hivetalk-Authorization` headers and inspect
 * what was signed.
 */
function setupSigner() {
  const signed = [];
  globalThis.window = globalThis.window ?? {};
  globalThis.window.__TAURI_INTERNALS__ = {
    invoke: async (command, input) => {
      if (command === "sign_event") {
        signed.push(input);
        return JSON.stringify({
          id: "0".repeat(64),
          pubkey: "a".repeat(64),
          created_at: input.createdAt ?? 1,
          kind: input.kind,
          tags: input.tags,
          content: input.content,
          sig: "b".repeat(128),
        });
      }
      throw new Error(`unexpected Tauri command: ${command}`);
    },
  };
  return signed;
}

function teardown() {
  delete globalThis.window.__TAURI_INTERNALS__;
  globalThis.fetch = undefined;
}

/** Decode a `Nostr <base64(json)>` auth header back to the event. */
function decodeAuth(headerValue) {
  return JSON.parse(
    Buffer.from(headerValue.replace(/^Nostr /, ""), "base64").toString("utf8"),
  );
}

function tag(event, name) {
  return event.tags.find((t) => t[0] === name)?.[1];
}

/** A fetch stub that answers the challenge call, then records the real call. */
function withChallengeAndCapture(realResponse, run) {
  const captured = {};
  globalThis.fetch = async (url, init = {}) => {
    if (url.endsWith("/meetings/auth/challenge")) {
      captured.challenge = { url, init };
      return new Response(
        JSON.stringify({
          challenge: "challenge-jwt",
          nonce: "nonce-123",
          expires_at: "2099",
          domain: "l402relay.exe.xyz",
        }),
        { status: 200 },
      );
    }
    captured.call = { url, init };
    return realResponse;
  };
  return Promise.resolve(run(captured)).finally(teardown);
}

test("subscribe signs two events with different `u` tags (relay vs upstream)", async () => {
  const signed = setupSigner();
  await withChallengeAndCapture(
    new Response(
      JSON.stringify({ intent_id: "i1", bolt11: "lnbc1", amount_sats: 10 }),
      { status: 201 },
    ),
    async (captured) => {
      const intent = await subscribe(RELAY_WS, CAP, "bulk10_1y");
      assert.equal(intent.intent_id, "i1");

      // buzz-relay NIP-98 on the outgoing Authorization header.
      const buzzEvent = decodeAuth(captured.call.init.headers.Authorization);
      assert.equal(
        tag(buzzEvent, "u"),
        "https://relay.example/meetings/subscribe",
      );
      assert.equal(tag(buzzEvent, "method"), "POST");
      assert.ok(tag(buzzEvent, "payload"), "buzz POST carries a payload tag");

      // HiveTalk signature on X-Hivetalk-Authorization.
      const htEvent = decodeAuth(
        captured.call.init.headers["X-Hivetalk-Authorization"],
      );
      assert.equal(
        tag(htEvent, "u"),
        "https://l402relay.exe.xyz/api/subscribe",
      );
      assert.equal(tag(htEvent, "action"), "subscribe");
      assert.equal(tag(htEvent, "nonce"), "nonce-123");
      assert.equal(
        captured.call.init.headers["X-Hivetalk-Challenge"],
        "challenge-jwt",
      );

      // Raw body forwarded verbatim.
      assert.equal(
        captured.call.init.body,
        JSON.stringify({ plan: "bulk10_1y" }),
      );
      // Three signatures: buzz NIP-98 for the challenge GET, the HiveTalk
      // action signature, and buzz NIP-98 for the subscribe POST itself.
      assert.equal(signed.length, 3);
    },
  );
});

test("subscribe on a 402 throws a typed subscription_required error", async () => {
  setupSigner();
  await withChallengeAndCapture(
    new Response(JSON.stringify({ reason: "subscription_required" }), {
      status: 402,
    }),
    async () => {
      await assert.rejects(subscribe(RELAY_WS, CAP, "p"), (err) => {
        assert.equal(err.kind, "subscription_required");
        assert.equal(err.status, 402);
        return true;
      });
    },
  );
});

test("subscribe on a 409 attaches the pending invoice", async () => {
  setupSigner();
  await withChallengeAndCapture(
    new Response(
      JSON.stringify({
        reason: "pending_invoice",
        intent_id: "old",
        bolt11: "lnbcOLD",
        amount_sats: 10,
      }),
      { status: 409 },
    ),
    async () => {
      await assert.rejects(subscribe(RELAY_WS, CAP, "p"), (err) => {
        assert.equal(err.kind, "pending_invoice");
        assert.equal(err.pendingInvoice.bolt11, "lnbcOLD");
        return true;
      });
    },
  );
});

test("subscribe resolves the invoice HiveTalk returns as an L402 402", async () => {
  setupSigner();
  // Captured live from `POST https://l402relay.exe.xyz/api/subscribe` on
  // 2026-09-04: the deployment answers with the L402 challenge status instead
  // of the `201` the previous provider sent, carrying the same intent body.
  await withChallengeAndCapture(
    new Response(
      JSON.stringify({
        intent_id: "i402",
        plan: "bulk10_1y",
        amount_sats: 360,
        bolt11: "lnbc3600n1...",
        payment_hash: "ph",
        expires_at: 1893456000,
        status: "pending",
      }),
      {
        status: 402,
        headers: {
          "www-authenticate": 'L402 macaroon="Ag...", invoice="lnbc3600n1..."',
        },
      },
    ),
    async () => {
      const intent = await subscribe(RELAY_WS, CAP, "bulk10_1y");
      assert.equal(intent.intent_id, "i402");
      assert.equal(intent.bolt11, "lnbc3600n1...");
      assert.equal(intent.status, "pending");
    },
  );
});

test("registerRoom keeps a 402 without an invoice on the hosting path", async () => {
  setupSigner();
  // `register-room` and `get-token` also answer 402, but with `plans[]` and no
  // BOLT11 — the L402 rescue must not swallow those into a success.
  await withChallengeAndCapture(
    new Response(
      JSON.stringify({ reason: "subscription_required", plans: [] }),
      { status: 402 },
    ),
    async () => {
      await assert.rejects(registerRoom(RELAY_WS, CAP, "room"), (err) => {
        assert.equal(err.kind, "subscription_required");
        assert.equal(err.status, 402);
        assert.equal(err.pendingInvoice, undefined);
        return true;
      });
    },
  );
});

test("getPaymentStatus signs the `u` tag with the id query string on both events", async () => {
  setupSigner();
  await withChallengeAndCapture(
    new Response(JSON.stringify({ intent_id: "i1", status: "settled" }), {
      status: 200,
    }),
    async (captured) => {
      await getPaymentStatus(RELAY_WS, CAP, "i1");
      assert.ok(captured.call.url.endsWith("/meetings/payment/status?id=i1"));

      const buzzEvent = decodeAuth(captured.call.init.headers.Authorization);
      assert.equal(
        tag(buzzEvent, "u"),
        "https://relay.example/meetings/payment/status?id=i1",
      );
      assert.equal(tag(buzzEvent, "method"), "GET");
      assert.equal(
        buzzEvent.tags.some((t) => t[0] === "payload"),
        false,
        "signed GET carries no payload tag",
      );

      const htEvent = decodeAuth(
        captured.call.init.headers["X-Hivetalk-Authorization"],
      );
      assert.equal(
        tag(htEvent, "u"),
        "https://l402relay.exe.xyz/api/payment/status?id=i1",
      );
      assert.equal(tag(htEvent, "action"), "payment-status");
    },
  );
});

test("listRooms sends only the buzz NIP-98 header — no HiveTalk headers", async () => {
  setupSigner();
  globalThis.fetch = async (url, init = {}) => {
    assert.ok(url.endsWith("/meetings/list-rooms"));
    assert.ok(init.headers.Authorization.startsWith("Nostr "));
    assert.equal(init.headers["X-Hivetalk-Authorization"], undefined);
    assert.equal(init.headers["X-Hivetalk-Challenge"], undefined);
    return new Response(JSON.stringify([{ name: "r", numParticipants: 2 }]), {
      status: 200,
    });
  };
  try {
    const rooms = await listRooms(RELAY_WS);
    assert.equal(rooms[0].name, "r");
  } finally {
    teardown();
  }
});

test("getMeetingToken embeds a body-signed event with the upstream `u` tag", async () => {
  setupSigner();
  let captured;
  globalThis.fetch = async (url, init = {}) => {
    captured = { url, init };
    return new Response(
      JSON.stringify({ token: "livekit-jwt", url: "wss://sfu.livekit.cloud" }),
      { status: 200 },
    );
  };
  try {
    const token = await getMeetingToken(
      RELAY_WS,
      CAP,
      "buzz-meet-1",
      "Alice",
      "a".repeat(64),
    );
    assert.equal(token.url, "wss://sfu.livekit.cloud");
    assert.ok(captured.url.endsWith("/meetings/get-token"));
    assert.equal(captured.init.headers["X-Hivetalk-Authorization"], undefined);

    const body = JSON.parse(captured.init.body);
    assert.equal(body.roomName, "buzz-meet-1");
    assert.equal(body.participantName, "Alice");
    assert.equal(typeof body.attributes.signed_event, "string");
    const inner = JSON.parse(body.attributes.signed_event);
    assert.equal(tag(inner, "u"), "https://l402relay.exe.xyz/api/get-token");
    assert.ok(Math.abs(inner.created_at - Math.floor(Date.now() / 1000)) < 300);
  } finally {
    teardown();
  }
});

test("moderateRoom forwards a Bearer LiveKit JWT and no challenge header", async () => {
  setupSigner();
  let captured;
  globalThis.fetch = async (url, init = {}) => {
    captured = { url, init };
    return new Response(null, { status: 204 });
  };
  try {
    // The payload comes from the same builder the host controls use — a
    // literal here is what let this test pass on a shape HiveTalk rejects.
    await moderateRoom(
      RELAY_WS,
      "kick-user",
      "livekit-jwt",
      participantActionPayload("buzz-meet-spike", "bob"),
    );
    assert.ok(captured.url.endsWith("/meetings/kick-user"));
    assert.equal(
      captured.init.headers["X-Hivetalk-Authorization"],
      "Bearer livekit-jwt",
    );
    assert.equal(captured.init.headers["X-Hivetalk-Challenge"], undefined);
    // HiveTalk's `ParticipantAction` schema, verbatim on the wire.
    assert.equal(
      captured.init.body,
      JSON.stringify({
        roomName: "buzz-meet-spike",
        participantIdentity: "bob",
      }),
    );
  } finally {
    teardown();
  }
});

test("registerRoom sends HiveTalk's snake_case room_name", async () => {
  setupSigner();
  await withChallengeAndCapture(
    new Response(
      JSON.stringify({
        room_id: "r1",
        room_name: "team-standup",
        pubkey: "a".repeat(64),
      }),
      { status: 201 },
    ),
    async (captured) => {
      const room = await registerRoom(RELAY_WS, CAP, "team-standup");
      assert.equal(room.room_name, "team-standup");
      // `{ roomName }` is what the integration notes said; the live API and
      // openapi.yaml both require `room_name` and 400 on anything else.
      assert.deepEqual(JSON.parse(captured.call.init.body), {
        room_name: "team-standup",
      });
    },
  );
});

test("listRoomsByPubkey normalizes registered rooms onto `name`", async () => {
  setupSigner();
  globalThis.fetch = async () =>
    new Response(
      JSON.stringify([
        { room_name: "Celestial  Solace", room_id: "r1", locked: true },
        { room_id: "no-name" },
      ]),
      { status: 200 },
    );
  try {
    const rooms = await listRoomsByPubkey(RELAY_WS, "f".repeat(64));
    assert.equal(rooms.length, 1, "the nameless entry is dropped");
    assert.equal(rooms[0].name, "Celestial  Solace");
    assert.equal(rooms[0].locked, true);
  } finally {
    teardown();
  }
});
