import { SELF } from "cloudflare:test";
import {
  finalizeEvent,
  generateSecretKey,
  getPublicKey,
  type Event,
} from "nostr-tools";
import { describe, expect, it } from "vitest";
import { KIND_BEACON_PULSE } from "../../src/pulse";
import {
  hexToBytes,
  TEST_PEER_SECRET_HEX,
  TEST_READER_SECRET_HEX,
} from "../replication/peer-fixture";
import {
  TEST_OWNER_SECRET_HEX,
  TEST_PULSE_NODE_LABEL,
  TEST_WITNESS_SECRET_HEX,
} from "./fixture";

const KIND_HTTP_AUTH = 27_235;
const KIND_RELAY_AUTH = 22_242;
const KIND_SYNC_DECLARATION = 30_700;

const OWNER_SECRET = hexToBytes(TEST_OWNER_SECRET_HEX);
const READER_SECRET = hexToBytes(TEST_READER_SECRET_HEX);
const PEER_SECRET = hexToBytes(TEST_PEER_SECRET_HEX);
const WITNESS_PUBKEY = getPublicKey(hexToBytes(TEST_WITNESS_SECRET_HEX));

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    bytes.buffer as ArrayBuffer,
  );
  return Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}

async function authedPost(
  secretKey: Uint8Array,
  url: string,
  body: unknown,
): Promise<Response> {
  const serialized = JSON.stringify(body);
  const payload = await sha256Hex(new TextEncoder().encode(serialized));
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
  return SELF.fetch(url, {
    method: "POST",
    headers: {
      authorization: `Nostr ${btoa(JSON.stringify(proof))}`,
      "content-type": "application/json",
    },
    body: serialized,
  });
}

function ownerDeclaration(dTag: string, granteePubkey: string): Event {
  return JSON.parse(
    JSON.stringify(
      finalizeEvent(
        {
          kind: KIND_SYNC_DECLARATION,
          created_at: Math.floor(Date.now() / 1000),
          tags: [
            ["d", dTag],
            ["n", TEST_PULSE_NODE_LABEL],
            ["p", granteePubkey],
          ],
          content: JSON.stringify({
            status: "active",
            principal: "did:example:reader-node",
          }),
        },
        OWNER_SECRET,
      ),
    ),
  ) as Event;
}

async function queryPulses(
  secretKey: Uint8Array,
  origin: string,
): Promise<Event[]> {
  const response = await authedPost(secretKey, `${origin}/query`, [
    { kinds: [KIND_BEACON_PULSE] },
  ]);
  expect(response.status).toBe(200);
  return (await response.json()) as Event[];
}

function nextFrame(socket: WebSocket): Promise<unknown[]> {
  return new Promise((resolve) => {
    socket.addEventListener(
      "message",
      (message) => resolve(JSON.parse(String(message.data)) as unknown[]),
      { once: true },
    );
  });
}

async function openAuthenticatedSocket(
  secretKey: Uint8Array,
  origin: string,
): Promise<WebSocket> {
  const response = await SELF.fetch(`${origin}/`, {
    headers: { Upgrade: "websocket" },
  });
  const socket = response.webSocket;
  if (socket === null) {
    throw new Error("expected WebSocket upgrade response");
  }
  socket.accept();
  const challengeFrame = await nextFrame(socket);
  expect(challengeFrame[0]).toBe("AUTH");
  const challenge = String(challengeFrame[1]);
  const auth = finalizeEvent(
    {
      kind: KIND_RELAY_AUTH,
      created_at: Math.floor(Date.now() / 1000),
      tags: [
        ["relay", `${origin.replace(/^http/, "ws")}/`],
        ["challenge", challenge],
      ],
      content: "",
    },
    secretKey,
  );
  const accepted = nextFrame(socket);
  socket.send(JSON.stringify(["AUTH", auth]));
  expect(await accepted).toEqual(["OK", auth.id, true, "authenticated"]);
  return socket;
}

describe("Beacon pulse standing under required identity", () => {
  it("shows the owner the pulse with its agreement heads", async () => {
    const origin = "https://pulse-standing-owner.example";
    const declaration = ownerDeclaration(
      "read/laptop-test/sovereign",
      getPublicKey(READER_SECRET),
    );
    const submitted = await authedPost(
      OWNER_SECRET,
      `${origin}/events`,
      declaration,
    );
    expect(await submitted.json()).toMatchObject({ message: "stored" });

    const pulses = await queryPulses(OWNER_SECRET, origin);
    expect(pulses).toHaveLength(1);
    const pulse = pulses[0];
    expect(pulse.pubkey).toBe(WITNESS_PUBKEY);
    expect(pulse.tags).toContainEqual(["n", TEST_PULSE_NODE_LABEL]);

    const content = JSON.parse(pulse.content) as {
      label: string;
      journal: { sequence: number; head: string | null };
      agreements: Record<string, string>;
      coherence: { governance: Record<string, string> };
    };
    expect(content.label).toBe(TEST_PULSE_NODE_LABEL);
    expect(content.journal).toEqual({ sequence: 1, head: declaration.id });
    expect(content.agreements).toEqual({
      "read/laptop-test/sovereign": declaration.id,
    });
    expect(content.coherence.governance).toEqual({
      peers: "bootstrap",
      readers: "journal",
      streams: "bootstrap",
    });
  });

  it("grants standing to declared readers and peers", async () => {
    const origin = "https://pulse-standing-parties.example";
    expect(await queryPulses(READER_SECRET, origin)).toHaveLength(1);
    expect(await queryPulses(PEER_SECRET, origin)).toHaveLength(1);
  });

  it("denies the pulse to an authenticated stranger", async () => {
    const origin = "https://pulse-standing-stranger.example";
    expect(await queryPulses(generateSecretKey(), origin)).toEqual([]);
  });

  it("witnesses authenticated socket sessions without connection metadata", async () => {
    const origin = "https://pulse-standing-sessions.example";
    const ownerSocket = await openAuthenticatedSocket(OWNER_SECRET, origin);
    const peerSocket = await openAuthenticatedSocket(PEER_SECRET, origin);

    const pulse = (await queryPulses(OWNER_SECRET, origin))[0];
    const content = JSON.parse(pulse.content) as {
      coherence: {
        sessions: { count: number; principals: string[] };
      };
    };
    expect(content.coherence.sessions).toEqual({
      count: 2,
      principals: [
        getPublicKey(OWNER_SECRET),
        getPublicKey(PEER_SECRET),
      ].sort(),
    });

    ownerSocket.close(1000, "done");
    peerSocket.close(1000, "done");
  });
});
