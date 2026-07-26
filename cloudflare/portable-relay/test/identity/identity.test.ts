import { env } from "cloudflare:workers";
import { evictDurableObject, SELF } from "cloudflare:test";
import { finalizeEvent, generateSecretKey, getPublicKey } from "nostr-tools";
import { describe, expect, it } from "vitest";
import { verifyNip42At } from "../../src/identity";
import identityFixture from "../../../../specs/fixtures/portable-relay/identity-v0.1.json";

const KIND_AUTH = 22_242;
const KIND_HTTP_AUTH = 27_235;
const KIND_GIFT_WRAP = 1_059;

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    bytes.buffer as ArrayBuffer,
  );
  return Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}

async function nip98Header(
  secretKey: Uint8Array,
  url: string,
  body: string,
): Promise<string> {
  const payload = await sha256Hex(new TextEncoder().encode(body));
  const proof = finalizeEvent(
    {
      kind: KIND_HTTP_AUTH,
      created_at: Math.floor(Date.now() / 1000),
      tags: [
        ["u", url],
        ["method", "POST"],
        ["payload", payload],
        ["nonce", crypto.randomUUID()],
      ],
      content: "",
    },
    secretKey,
  );
  return `Nostr ${btoa(JSON.stringify(proof))}`;
}

async function authedPost(
  secretKey: Uint8Array,
  url: string,
  body: unknown,
): Promise<Response> {
  const serialized = JSON.stringify(body);
  return SELF.fetch(url, {
    method: "POST",
    headers: {
      authorization: await nip98Header(secretKey, url, serialized),
      "content-type": "application/json",
    },
    body: serialized,
  });
}

function signedNote(secretKey: Uint8Array, content: string) {
  return JSON.parse(
    JSON.stringify(
      finalizeEvent(
        {
          kind: 1,
          created_at: Math.floor(Date.now() / 1000),
          tags: [],
          content,
        },
        secretKey,
      ),
    ),
  );
}

describe("fixed NIP-42 conformance vector", () => {
  const proof = identityFixture.authentication_evidence;

  it("binds the signing principal for the fixture audience", () => {
    const verified = verifyNip42At(
      // biome-ignore lint/suspicious/noExplicitAny: fixture event shape is validated by the verifier
      proof.event as any,
      proof.challenge,
      proof.audience,
      proof.evaluation_time,
    );
    expect(verified).toMatchObject({
      ok: true,
      pubkey: proof.expected.principal_id,
    });
  });

  it("rejects evidence bound to another audience", () => {
    const verified = verifyNip42At(
      // biome-ignore lint/suspicious/noExplicitAny: fixture event shape is validated by the verifier
      proof.event as any,
      proof.challenge,
      "wss://different.example/",
      proof.evaluation_time,
    );
    expect(verified).toEqual({ ok: false, code: "audience_mismatch" });
  });

  it("rejects expired evidence", () => {
    const verified = verifyNip42At(
      // biome-ignore lint/suspicious/noExplicitAny: fixture event shape is validated by the verifier
      proof.event as any,
      proof.challenge,
      proof.audience,
      proof.evaluation_time + 3_600,
    );
    expect(verified).toEqual({ ok: false, code: "evidence_expired" });
  });
});

describe("secured HTTP surface", () => {
  it("requires authentication and binds the author", async () => {
    const origin = "https://id-http.example";
    const author = generateSecretKey();
    const other = generateSecretKey();
    const event = signedNote(author, "attributable");

    const anonymous = await SELF.fetch(`${origin}/events`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(event),
    });
    expect(anonymous.status).toBe(401);
    expect(await anonymous.json()).toMatchObject({
      code: "authentication_required",
    });

    const accepted = await authedPost(author, `${origin}/events`, event);
    expect(accepted.status).toBe(200);
    expect(await accepted.json()).toMatchObject({ message: "stored" });

    const mismatched = await authedPost(
      other,
      `${origin}/events`,
      signedNote(author, "not yours"),
    );
    expect(mismatched.status).toBe(403);
    expect(await mismatched.json()).toMatchObject({ code: "author_mismatch" });
  });

  it("rejects a replayed proof even after eviction", async () => {
    const origin = "https://id-replay.example";
    const author = generateSecretKey();
    const event = signedNote(author, "one proof, one use");
    const serialized = JSON.stringify(event);
    const url = `${origin}/events`;
    const header = await nip98Header(author, url, serialized);
    const request = () =>
      SELF.fetch(url, {
        method: "POST",
        headers: {
          authorization: header,
          "content-type": "application/json",
        },
        body: serialized,
      });

    expect((await request()).status).toBe(200);

    await evictDurableObject(env.RELAY_NODES.getByName("id-replay.example"));

    const replayed = await request();
    expect(replayed.status).toBe(401);
    expect(await replayed.json()).toMatchObject({ code: "replay_detected" });
  });

  it("never journals authentication events", async () => {
    const origin = "https://id-journal.example";
    const author = generateSecretKey();
    const authEvent = JSON.parse(
      JSON.stringify(
        finalizeEvent(
          {
            kind: KIND_AUTH,
            created_at: Math.floor(Date.now() / 1000),
            tags: [
              ["challenge", "c".repeat(64)],
              ["relay", "wss://id-journal.example/"],
            ],
            content: "",
          },
          author,
        ),
      ),
    );

    const submitted = await authedPost(author, `${origin}/events`, authEvent);
    expect(submitted.status).toBe(403);
    expect(await submitted.json()).toMatchObject({ code: "scope_denied" });

    const history = await authedPost(author, `${origin}/query`, [
      { kinds: [KIND_AUTH, KIND_HTTP_AUTH], authors: [getPublicKey(author)] },
    ]);
    expect(await history.json()).toEqual([]);
  });

  it("filters protected events from query and count per reader", async () => {
    const origin = "https://id-disclosure.example";
    const sender = generateSecretKey();
    const recipient = generateSecretKey();
    const attacker = generateSecretKey();
    const giftWrap = JSON.parse(
      JSON.stringify(
        finalizeEvent(
          {
            kind: KIND_GIFT_WRAP,
            created_at: Math.floor(Date.now() / 1000),
            tags: [["p", getPublicKey(recipient)]],
            content: "ciphertext",
          },
          generateSecretKey(),
        ),
      ),
    );

    const stored = await authedPost(sender, `${origin}/events`, giftWrap);
    expect(stored.status).toBe(200);

    const filters = [{ ids: [giftWrap.id] }];
    const recipientQuery = await authedPost(
      recipient,
      `${origin}/query`,
      filters,
    );
    expect(await recipientQuery.json()).toEqual([giftWrap]);
    const attackerQuery = await authedPost(
      attacker,
      `${origin}/query`,
      filters,
    );
    expect(await attackerQuery.json()).toEqual([]);

    const recipientCount = await authedPost(
      recipient,
      `${origin}/count`,
      filters,
    );
    expect(await recipientCount.json()).toEqual({ count: 1 });
    const attackerCount = await authedPost(
      attacker,
      `${origin}/count`,
      filters,
    );
    expect(await attackerCount.json()).toEqual({ count: 0 });
  });
});

describe("secured WebSocket surface", () => {
  async function openSecuredSocket(origin: string) {
    const response = await SELF.fetch(`${origin}/`, {
      headers: { Upgrade: "websocket" },
    });
    const socket = response.webSocket;
    if (socket === null) {
      throw new Error("expected WebSocket upgrade response");
    }
    socket.accept();
    const frames: unknown[][] = [];
    const waiters: {
      match: (frame: unknown[]) => boolean;
      resolve: (frame: unknown[]) => void;
    }[] = [];
    socket.addEventListener("message", (event) => {
      const frame = JSON.parse(String(event.data)) as unknown[];
      const index = waiters.findIndex((waiter) => waiter.match(frame));
      if (index >= 0) {
        waiters.splice(index, 1)[0].resolve(frame);
      } else {
        frames.push(frame);
      }
    });
    const next = (match: (frame: unknown[]) => boolean) => {
      const buffered = frames.findIndex((frame) => match(frame));
      if (buffered >= 0) {
        return Promise.resolve(frames.splice(buffered, 1)[0]);
      }
      return new Promise<unknown[]>((resolve) =>
        waiters.push({ match, resolve }),
      );
    };
    return {
      socket,
      next,
      send: (frame: unknown[]) => socket.send(JSON.stringify(frame)),
    };
  }

  it("challenges, authenticates, and binds the event author", async () => {
    const origin = "https://id-ws.example";
    const keys = generateSecretKey();
    const session = await openSecuredSocket(origin);

    const challengeFrame = await session.next((frame) => frame[0] === "AUTH");
    const challenge = challengeFrame[1] as string;

    const early = signedNote(keys, "too early");
    session.send(["EVENT", early]);
    const denied = await session.next(
      (frame) => frame[0] === "OK" && frame[1] === early.id,
    );
    expect(denied[2]).toBe(false);
    expect(denied[3]).toBe("authentication_required");

    const authEvent = JSON.parse(
      JSON.stringify(
        finalizeEvent(
          {
            kind: KIND_AUTH,
            created_at: Math.floor(Date.now() / 1000),
            tags: [
              ["challenge", challenge],
              ["relay", "wss://id-ws.example/"],
            ],
            content: "",
          },
          keys,
        ),
      ),
    );
    session.send(["AUTH", authEvent]);
    const authOk = await session.next(
      (frame) => frame[0] === "OK" && frame[1] === authEvent.id,
    );
    expect(authOk[2]).toBe(true);

    const note = signedNote(keys, "over secured WebSocket");
    session.send(["EVENT", note]);
    const stored = await session.next(
      (frame) => frame[0] === "OK" && frame[1] === note.id,
    );
    expect(stored[2]).toBe(true);
    expect(stored[3]).toBe("stored");

    const journal = await authedPost(keys, `${origin}/query`, [
      { kinds: [KIND_AUTH] },
    ]);
    expect(await journal.json()).toEqual([]);
    session.socket.close(1000, "done");
  });

  it("keeps the authenticated principal across hibernation", async () => {
    const origin = "https://id-ws-hibernate.example";
    const keys = generateSecretKey();
    const session = await openSecuredSocket(origin);
    const challenge = (
      await session.next((frame) => frame[0] === "AUTH")
    )[1] as string;

    const authEvent = JSON.parse(
      JSON.stringify(
        finalizeEvent(
          {
            kind: KIND_AUTH,
            created_at: Math.floor(Date.now() / 1000),
            tags: [
              ["challenge", challenge],
              ["relay", "wss://id-ws-hibernate.example/"],
            ],
            content: "",
          },
          keys,
        ),
      ),
    );
    session.send(["AUTH", authEvent]);
    await session.next(
      (frame) => frame[0] === "OK" && frame[1] === authEvent.id,
    );

    session.send([
      "REQ",
      "live",
      { kinds: [1], authors: [getPublicKey(keys)] },
    ]);
    await session.next((frame) => frame[0] === "EOSE" && frame[1] === "live");

    await evictDurableObject(
      env.RELAY_NODES.getByName("id-ws-hibernate.example"),
      { webSockets: "hibernate" },
    );

    const note = signedNote(keys, "delivered after hibernation");
    const submitted = await authedPost(keys, `${origin}/events`, note);
    expect(submitted.status).toBe(200);
    const live = await session.next(
      (frame) => frame[0] === "EVENT" && frame[1] === "live",
    );
    expect(live[2]).toMatchObject({ id: note.id });
    session.socket.close(1000, "done");
  });
});
