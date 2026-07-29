import { SELF } from "cloudflare:test";
import {
  finalizeEvent,
  generateSecretKey,
  getPublicKey,
  verifyEvent,
  type Event,
} from "nostr-tools";
import { describe, expect, it } from "vitest";
import { KIND_BEACON_PULSE, ADAPTER_ID } from "../src/pulse";
import { hexToBytes } from "./replication/peer-fixture";
import { TEST_WITNESS_SECRET_HEX } from "./pulse/fixture";

const WITNESS_PUBKEY = getPublicKey(hexToBytes(TEST_WITNESS_SECRET_HEX));

interface PulseContent {
  node: string;
  label: string;
  adapter: string;
  journal: { sequence: number; head: string | null };
  previous: string | null;
  checkpoints: Record<string, string>;
  agreements: Record<string, string>;
  coherence: { governance: Record<string, string> };
}

function pulseContent(event: Event): PulseContent {
  return JSON.parse(event.content) as PulseContent;
}

function signedNote(secretKey: Uint8Array, content: string): Event {
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
  ) as Event;
}

function postJson(url: string, body: unknown): Promise<Response> {
  return SELF.fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}

async function queryEvents(origin: string, filters: unknown): Promise<Event[]> {
  const response = await postJson(`${origin}/query`, filters);
  expect(response.status).toBe(200);
  return (await response.json()) as Event[];
}

async function openWebSocket(origin: string): Promise<WebSocket> {
  const response = await SELF.fetch(`${origin}/`, {
    headers: { Upgrade: "websocket" },
  });
  const socket = response.webSocket;
  if (socket === null) {
    throw new Error("expected WebSocket upgrade response");
  }
  socket.accept();
  return socket;
}

function collectFramesUntil(
  socket: WebSocket,
  complete: (frame: unknown[]) => boolean,
): Promise<unknown[][]> {
  return new Promise((resolve) => {
    const frames: unknown[][] = [];
    const onMessage = (event: MessageEvent) => {
      const frame = JSON.parse(String(event.data)) as unknown[];
      frames.push(frame);
      if (complete(frame)) {
        socket.removeEventListener("message", onMessage);
        resolve(frames);
      }
    };
    socket.addEventListener("message", onMessage);
  });
}

describe("Beacon pulse on an open node", () => {
  it("synthesizes a signed pulse only when explicitly requested", async () => {
    const origin = "https://pulse-basic.example";
    const note = signedNote(generateSecretKey(), "witnessed");
    const submitted = await postJson(`${origin}/events`, note);
    expect(await submitted.json()).toMatchObject({ message: "stored" });

    const pulses = await queryEvents(origin, [{ kinds: [KIND_BEACON_PULSE] }]);
    expect(pulses).toHaveLength(1);
    const pulse = pulses[0];
    expect(pulse.kind).toBe(KIND_BEACON_PULSE);
    expect(pulse.pubkey).toBe(WITNESS_PUBKEY);
    expect(verifyEvent(pulse)).toBe(true);
    expect(pulse.tags).toContainEqual(["role", "rendezvous"]);

    const content = pulseContent(pulse);
    expect(content.node).toBe("pulse-basic.example");
    expect(content.adapter).toBe(ADAPTER_ID);
    expect(content.journal).toEqual({ sequence: 1, head: note.id });
    expect(content.previous).toBeNull();
    expect(content.coherence.governance).toEqual({
      peers: "bootstrap",
      readers: "bootstrap",
      streams: "bootstrap",
    });

    // A filter that does not name the pulse kind never surfaces it.
    expect(await queryEvents(origin, [{ kinds: [1] }])).toEqual([note]);
    expect(await queryEvents(origin, [{ ids: [note.id] }])).toEqual([note]);
  });

  it("advances the witnessed chain across journal transitions", async () => {
    const origin = "https://pulse-chain.example";
    const author = generateSecretKey();
    const first = signedNote(author, "first");
    const second = signedNote(author, "second");
    await postJson(`${origin}/events`, first);
    await postJson(`${origin}/events`, second);

    const pulses = await queryEvents(origin, [{ kinds: [KIND_BEACON_PULSE] }]);
    expect(pulses).toHaveLength(1);
    const content = pulseContent(pulses[0]);
    expect(content.journal).toEqual({ sequence: 2, head: second.id });
    expect(content.previous).toBe(first.id);
  });

  it("delivers a live pulse to subscribers on journal transitions", async () => {
    const origin = "https://pulse-live.example";
    const socket = await openWebSocket(origin);
    const eoseFrames = await (async () => {
      const promise = collectFramesUntil(
        socket,
        (frame) => frame[0] === "EOSE",
      );
      socket.send(
        JSON.stringify(["REQ", "beacon", { kinds: [KIND_BEACON_PULSE] }]),
      );
      return promise;
    })();
    // The initial dump already carries a synthesized pulse of the (empty)
    // journal — subscribing IS asking.
    expect(eoseFrames).toHaveLength(2);
    const initial = eoseFrames[0] as [string, string, Event];
    expect(initial[0]).toBe("EVENT");
    expect(pulseContent(initial[2]).journal).toEqual({
      sequence: 0,
      head: null,
    });

    const livePromise = collectFramesUntil(
      socket,
      (frame) => frame[0] === "EVENT",
    );
    const note = signedNote(generateSecretKey(), "transition");
    await postJson(`${origin}/events`, note);
    const liveFrames = await livePromise;
    const live = liveFrames[liveFrames.length - 1] as [string, string, Event];
    expect(live[2].kind).toBe(KIND_BEACON_PULSE);
    expect(live[2].pubkey).toBe(WITNESS_PUBKEY);
    expect(pulseContent(live[2]).journal).toEqual({
      sequence: 1,
      head: note.id,
    });
    socket.close(1000, "done");
  });

  it("treats client-submitted pulse-kind events as ephemeral, never journaled", async () => {
    const origin = "https://pulse-foreign.example";
    const foreign = JSON.parse(
      JSON.stringify(
        finalizeEvent(
          {
            kind: KIND_BEACON_PULSE,
            created_at: Math.floor(Date.now() / 1000),
            tags: [],
            content: "{}",
          },
          generateSecretKey(),
        ),
      ),
    ) as Event;
    const submitted = await postJson(`${origin}/events`, foreign);
    expect(await submitted.json()).toMatchObject({ message: "ephemeral" });

    const pulses = await queryEvents(origin, [{ kinds: [KIND_BEACON_PULSE] }]);
    expect(pulses).toHaveLength(1);
    expect(pulses[0].pubkey).toBe(WITNESS_PUBKEY);
    expect(pulseContent(pulses[0]).journal).toEqual({
      sequence: 0,
      head: null,
    });
  });
});
