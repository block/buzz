import { env } from "cloudflare:workers";
import { evictDurableObject, SELF } from "cloudflare:test";
import { finalizeEvent, generateSecretKey } from "nostr-tools";
import { describe, expect, it } from "vitest";
import coreFixture from "../../../specs/fixtures/portable-relay/core-v0.1.json";
import signedEvent from "../../../specs/fixtures/local-relay/signed-message.json";

describe("portable relay Cloudflare boundary", () => {
  it("reports readiness without claiming core conformance", async () => {
    const response = await SELF.fetch("https://coherence.example/health");

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      status: "ok",
      adapter: "portable-relay-cloudflare-v0.1",
      implementation: "portable-core-candidate",
    });
  });

  it("routes equivalent authorities to one durable node", async () => {
    const first = await SELF.fetch("https://COHERENCE.EXAMPLE./query", {
      method: "POST",
      body: "[]",
    });
    const second = await SELF.fetch("https://coherence.example:443/count", {
      method: "POST",
      body: "[]",
    });

    expect(first.status).toBe(200);
    expect(second.status).toBe(200);

    const node = env.RELAY_NODES.getByName("coherence.example");
    await expect(node.describeNode()).resolves.toEqual({
      stableNodeKey: "coherence.example",
    });
  });

  it("isolates different stable node keys", async () => {
    const primary = env.RELAY_NODES.getByName("coherence.example");
    const isolated = env.RELAY_NODES.getByName("identity.example");

    await primary.initializeNode("coherence.example");
    await isolated.initializeNode("identity.example");

    await expect(primary.describeNode()).resolves.toEqual({
      stableNodeKey: "coherence.example",
    });
    await expect(isolated.describeNode()).resolves.toEqual({
      stableNodeKey: "identity.example",
    });
  });

  it("does not allow request content to select another node", async () => {
    const response = await SELF.fetch("https://body-boundary.example/events", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ stable_node_key: "body-override.example" }),
    });

    expect(response.status).toBe(400);
    await expect(
      env.RELAY_NODES.getByName("body-boundary.example").describeNode(),
    ).resolves.toBeNull();
    await expect(
      env.RELAY_NODES.getByName("body-override.example").describeNode(),
    ).resolves.toBeNull();

    const accepted = await postJson(
      "https://body-boundary.example/events",
      signedEvent,
    );
    expect(accepted.status).toBe(200);
    await expect(
      env.RELAY_NODES.getByName("body-boundary.example").describeNode(),
    ).resolves.toEqual({ stableNodeKey: "body-boundary.example" });
    await expect(
      env.RELAY_NODES.getByName("body-override.example").describeNode(),
    ).resolves.toBeNull();
  });

  it("recovers its stable node binding after eviction", async () => {
    const node = env.RELAY_NODES.getByName("coherence.example");
    await node.initializeNode("coherence.example");

    await evictDurableObject(node);

    const recovered = env.RELAY_NODES.getByName("coherence.example");
    await expect(recovered.describeNode()).resolves.toEqual({
      stableNodeKey: "coherence.example",
    });
  });

  it("preserves the shared signed-event vector through HTTP and eviction", async () => {
    const origin = "https://core-vector.example";
    const submitted = await postJson(`${origin}/events`, signedEvent);

    expect(submitted.status).toBe(200);
    expect(await submitted.json()).toEqual({
      event_id: coreFixture.event_id,
      accepted: coreFixture.operations.submit.expected.accepted,
      message: "stored",
    });

    const duplicate = await postJson(`${origin}/events`, signedEvent);
    expect(await duplicate.json()).toMatchObject({
      accepted: true,
      event_id: coreFixture.event_id,
      message: "duplicate",
    });

    const beforeEviction = await postJson(
      `${origin}/query`,
      coreFixture.operations.query.filters,
    );
    expect(await beforeEviction.json()).toEqual([signedEvent]);

    const count = await postJson(
      `${origin}/count`,
      coreFixture.operations.query.filters,
    );
    expect(await count.json()).toEqual({
      count: coreFixture.operations.count.expected,
    });

    await evictDurableObject(env.RELAY_NODES.getByName("core-vector.example"));

    const recovered = await postJson(
      `${origin}/query`,
      coreFixture.operations.query.filters,
    );
    expect(await recovered.json()).toEqual([signedEvent]);
  });

  it("rejects unsupported search without broadening the query", async () => {
    const origin = "https://unsupported-filter.example";
    await postJson(`${origin}/events`, signedEvent);

    const response = await postJson(`${origin}/query`, [
      { search: "owner-attested" },
    ]);

    expect(response.status).toBe(400);
    expect(await response.json()).toEqual({
      error: "invalid_request",
      message: "NIP-50 search is unsupported",
    });
  });

  it("rejects a tampered event without mutating queryable state", async () => {
    const origin = "https://tampered-vector.example";
    const tampered = { ...signedEvent, content: "tampered after signing" };

    const submission = await postJson(`${origin}/events`, tampered);
    expect(await submission.json()).toEqual({
      event_id: signedEvent.id,
      accepted: false,
      message: "invalid",
    });

    const query = await postJson(
      `${origin}/query`,
      coreFixture.operations.query.filters,
    );
    expect(await query.json()).toEqual([]);
  });

  it("preserves replacement and ephemeral decisions across eviction", async () => {
    const origin = "https://reducer.example";
    const secretKey = generateSecretKey();
    const newer = finalizeEvent(
      {
        kind: 10_000,
        created_at: 200,
        tags: [],
        content: "newer",
      },
      secretKey,
    );
    const older = finalizeEvent(
      {
        kind: 10_000,
        created_at: 100,
        tags: [],
        content: "older",
      },
      secretKey,
    );
    const ephemeral = finalizeEvent(
      {
        kind: 20_001,
        created_at: 300,
        tags: [],
        content: "live only",
      },
      secretKey,
    );

    expect(
      await (await postJson(`${origin}/events`, newer)).json(),
    ).toMatchObject({ accepted: true, message: "stored" });
    expect(
      await (await postJson(`${origin}/events`, older)).json(),
    ).toMatchObject({ accepted: true, message: "superseded" });
    expect(
      await (await postJson(`${origin}/events`, ephemeral)).json(),
    ).toMatchObject({ accepted: true, message: "ephemeral" });

    await evictDurableObject(env.RELAY_NODES.getByName("reducer.example"));

    expect(
      await (await postJson(`${origin}/events`, older)).json(),
    ).toMatchObject({ accepted: true, message: "duplicate" });
    const effective = await postJson(`${origin}/query`, [{ kinds: [10_000] }]);
    expect(await effective.json()).toEqual([JSON.parse(JSON.stringify(newer))]);
    const absentEphemeral = await postJson(`${origin}/query`, [
      { ids: [ephemeral.id] },
    ]);
    expect(await absentEphemeral.json()).toEqual([]);
  });

  it("serves matching history followed by EOSE over WebSocket", async () => {
    const origin = "https://ws-history.example";
    await postJson(`${origin}/events`, signedEvent);
    const socket = await openWebSocket(origin);
    const framesPromise = collectFramesUntil(
      socket,
      (frame) => frame[0] === "EOSE",
    );

    socket.send(
      JSON.stringify([
        "REQ",
        "history",
        coreFixture.operations.query.filters[0],
      ]),
    );

    const frames = await framesPromise;
    expect(frames).toEqual([
      ["EVENT", "history", signedEvent],
      ["EOSE", "history"],
    ]);
    socket.close(1000, "done");
  });

  it("preserves a SQLite-backed subscription across hibernation", async () => {
    const origin = "https://ws-hibernation.example";
    const socket = await openWebSocket(origin);
    const eosePromise = collectFramesUntil(
      socket,
      (frame) => frame[0] === "EOSE",
    );
    socket.send(
      JSON.stringify(["REQ", "live", coreFixture.operations.query.filters[0]]),
    );
    expect(await eosePromise).toEqual([["EOSE", "live"]]);

    await evictDurableObject(
      env.RELAY_NODES.getByName("ws-hibernation.example"),
      { webSockets: "hibernate" },
    );

    const livePromise = collectFramesUntil(
      socket,
      (frame) => frame[0] === "EVENT",
    );
    await postJson(`${origin}/events`, signedEvent);
    expect(await livePromise).toEqual([["EVENT", "live", signedEvent]]);
    socket.close(1000, "done");
  });

  it("accepts EVENT and returns OK over WebSocket", async () => {
    const origin = "https://ws-event.example";
    const socket = await openWebSocket(origin);
    const okPromise = collectFramesUntil(socket, (frame) => frame[0] === "OK");

    socket.send(JSON.stringify(["EVENT", signedEvent]));

    expect(await okPromise).toEqual([["OK", signedEvent.id, true, "stored"]]);
    const query = await postJson(
      `${origin}/query`,
      coreFixture.operations.query.filters,
    );
    expect(await query.json()).toEqual([signedEvent]);
    socket.close(1000, "done");
  });

  it("keeps accepting and delivering after another subscriber's socket closes", async () => {
    const origin = "https://ws-resilience.example";
    const closingSocket = await openWebSocket(origin);
    const closingEose = collectFramesUntil(
      closingSocket,
      (frame) => frame[0] === "EOSE",
    );
    closingSocket.send(
      JSON.stringify([
        "REQ",
        "doomed",
        coreFixture.operations.query.filters[0],
      ]),
    );
    await closingEose;

    const survivingSocket = await openWebSocket(origin);
    const survivingEose = collectFramesUntil(
      survivingSocket,
      (frame) => frame[0] === "EOSE",
    );
    survivingSocket.send(
      JSON.stringify(["REQ", "live", coreFixture.operations.query.filters[0]]),
    );
    await survivingEose;

    closingSocket.close(1000, "gone");
    const livePromise = collectFramesUntil(
      survivingSocket,
      (frame) => frame[0] === "EVENT",
    );
    const submitted = await postJson(`${origin}/events`, signedEvent);
    expect(submitted.status).toBe(200);
    expect(await submitted.json()).toMatchObject({ message: "stored" });
    expect(await livePromise).toEqual([["EVENT", "live", signedEvent]]);
    survivingSocket.close(1000, "done");
  });

  it("stops delivery after CLOSE and ignores non-matching filters", async () => {
    const origin = "https://ws-close.example";
    const socket = await openWebSocket(origin);
    const initialEose = collectFramesUntil(
      socket,
      (frame) => frame[0] === "EOSE",
    );
    socket.send(JSON.stringify(["REQ", "closed", { ids: [signedEvent.id] }]));
    await initialEose;

    socket.send(JSON.stringify(["CLOSE", "closed"]));
    const barrierEose = collectFramesUntil(
      socket,
      (frame) => frame[0] === "EOSE",
    );
    socket.send(JSON.stringify(["REQ", "barrier", { kinds: [2] }]));
    await barrierEose;

    const noFrame = expectNoFrame(socket);
    await postJson(`${origin}/events`, signedEvent);
    await noFrame;
    socket.close(1000, "done");
  });
});

function postJson(url: string, body: unknown): Promise<Response> {
  return SELF.fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
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

function expectNoFrame(socket: WebSocket): Promise<void> {
  return new Promise((resolve, reject) => {
    const onMessage = (event: MessageEvent) => {
      clearTimeout(timer);
      socket.removeEventListener("message", onMessage);
      reject(new Error(`unexpected WebSocket frame: ${String(event.data)}`));
    };
    const timer = setTimeout(() => {
      socket.removeEventListener("message", onMessage);
      resolve();
    }, 50);
    socket.addEventListener("message", onMessage);
  });
}
