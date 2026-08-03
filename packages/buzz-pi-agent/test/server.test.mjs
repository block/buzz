import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { PassThrough, Readable } from "node:stream";
import { test } from "node:test";
import { AcpServer, NdjsonWriter, SessionRegistry } from "../dist/index.js";
import {
  fakeHandle,
  MemoryWriter,
  silentLogger,
  testConfig,
} from "./helpers.mjs";

const CURRENT_LIFECYCLE_GENERATION = "a".repeat(64);

function harness(handleOverrides = {}, createOverrides = {}) {
  const handle = fakeHandle(handleOverrides);
  const calls = {
    create: [],
    dispose: [],
    get: [],
    reset: [],
    persistedEvents: [],
    acknowledgements: [],
    timeline: [],
  };
  const liveSessions = new Map();
  let createCount = 0;
  const registry = {
    async start() {},
    async shutdown() {},
    async create(options) {
      calls.create.push(options);
      createCount += 1;
      const sessionId =
        createOverrides.sessionIdFactory?.(createCount) ?? "ses_test";
      liveSessions.set(sessionId, options.conversationId);
      return {
        sessionId,
        handle,
        ...createOverrides,
        ...(options.conversationId === undefined
          ? {}
          : {
              lifecycleGeneration:
                createOverrides.lifecycleGeneration ??
                CURRENT_LIFECYCLE_GENERATION,
            }),
      };
    },
    get(sessionId) {
      calls.get.push(sessionId);
      return handle;
    },
    async disposeSession(sessionId, forget) {
      calls.dispose.push({ sessionId, forget });
      liveSessions.delete(sessionId);
      return true;
    },
    async commitConversationReset(conversationId, resetToken) {
      calls.reset.push({ conversationId, resetToken });
      for (const [sessionId, liveConversationId] of liveSessions) {
        if (liveConversationId === conversationId)
          liveSessions.delete(sessionId);
      }
      return { committed: true, alreadyCommitted: false };
    },
    hasSession(sessionId) {
      return liveSessions.has(sessionId);
    },
    conversationIdForSession(sessionId) {
      return liveSessions.get(sessionId);
    },
    conversationIdentityForSession(sessionId) {
      const conversationId = liveSessions.get(sessionId);
      return conversationId === undefined
        ? undefined
        : {
            conversationId,
            lifecycleGeneration:
              createOverrides.lifecycleGeneration ??
              CURRENT_LIFECYCLE_GENERATION,
          };
    },
    async persistConversationSessionEvent(
      conversationId,
      lifecycleGeneration,
      eventId,
      event,
    ) {
      if (createOverrides.persistError) throw createOverrides.persistError;
      calls.persistedEvents.push({
        conversationId,
        lifecycleGeneration,
        eventId,
        event,
      });
      calls.timeline.push({ type: "persist", eventId });
      return createOverrides.persistResult ?? true;
    },
    async listPendingSessionEvents() {
      return createOverrides.pendingEvents ?? [];
    },
    async acknowledgeSessionEvent(conversationId, eventId) {
      calls.acknowledgements.push({ conversationId, eventId });
    },
  };
  const writer = new MemoryWriter();
  const write = writer.write.bind(writer);
  writer.write = (message) => {
    calls.timeline.push({
      type: "write",
      method: message.method,
      eventId: message.params?.eventId,
    });
    write(message);
  };
  const server = new AcpServer(
    Readable.from([]),
    writer,
    registry,
    testConfig(),
    silentLogger,
  );
  return { server, writer, registry, handle, calls };
}

test("initialize advertises ACP v2 thread sessions, persistence, disposal, and reset tokens", async () => {
  const { server } = harness();
  const result = await server.handleRequest("initialize", {
    protocolVersion: 2,
  });
  assert.equal(result.protocolVersion, 2);
  assert.deepEqual(result._meta.buzz.threadSessions, {
    supported: true,
    persistence: true,
    dispose: true,
    resetToken: true,
    resetCommit: true,
  });
  assert.equal(result._meta.steering.supported, true);
  assert.deepEqual(result._meta.buzz.sessionEvents, {
    supported: true,
    durableReplay: true,
    ack: true,
    schemaVersion: 2,
  });
  await assert.rejects(
    () => server.handleRequest("initialize", { protocolVersion: 2 }),
    /initialize may only be called once/,
  );
});

test("session/new forwards the durable Buzz conversation and authenticated reset token", async () => {
  const { server, calls } = harness();
  await server.handleRequest("initialize", { protocolVersion: 2 });
  const result = await server.handleRequest("session/new", {
    cwd: "/tmp/project",
    systemPrompt: "Buzz system prompt",
    mcpServers: [],
    _meta: {
      sessionTitle: "Agent · #general",
      buzz: { conversationId: "channel:root", resetToken: "event-reset-1" },
    },
  });
  assert.deepEqual(calls.create[0], {
    cwd: "/tmp/project",
    systemPrompt: "Buzz system prompt",
    title: "Agent · #general",
    conversationId: "channel:root",
    resetToken: "event-reset-1",
  });
  assert.equal(result.sessionId, "ses_test");
  assert.equal(result._meta.buzz.contextLimitTokens, 150_000);
  assert.equal(result._meta.buzz.compactionThresholdTokens, 133_616);
  assert.equal(result._meta.buzz.skipRelayHistory, false);
  assert.equal(result.models.currentModelId, "provider/model");
});

test("session/new exposes a durable reset barrier to suppress relay history seeding", async () => {
  const { server } = harness(
    {},
    { resumedConversation: false, skipRelayHistory: true },
  );
  await server.handleRequest("initialize", { protocolVersion: 2 });
  const result = await server.handleRequest("session/new", {
    cwd: "/tmp/project",
    mcpServers: [],
    _meta: { buzz: { conversationId: "channel:root" } },
  });
  assert.equal(result._meta.buzz.resumedConversation, false);
  assert.equal(result._meta.buzz.skipRelayHistory, true);
});

test("session/set_config_option returns the complete stable ACP config list with applied values", async () => {
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "./fixtures/acp-set-config-option-response.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  const applied = [];
  const { server } = harness({
    models: [
      { id: "provider/model", name: "Model" },
      { id: "provider/alternate", name: "Alternate" },
    ],
    async setModel(modelId) {
      applied.push({ configId: "model", value: modelId });
    },
    async setThinkingLevel(level) {
      applied.push({ configId: "thinking", value: level });
    },
  });
  await server.handleRequest("initialize", { protocolVersion: 2 });

  assert.deepEqual(
    await server.handleRequest("session/set_config_option", {
      sessionId: "ses_test",
      configId: "model",
      value: "provider/alternate",
    }),
    fixture.model,
  );
  assert.deepEqual(
    await server.handleRequest("session/set_config_option", {
      sessionId: "ses_test",
      configId: "thinking",
      value: "high",
    }),
    fixture.thinking,
  );
  assert.deepEqual(applied, [
    { configId: "model", value: "provider/alternate" },
    { configId: "thinking", value: "high" },
  ]);
});

test("exact _buzz/session/dispose contract releases or forgets a Pi session", async () => {
  const { server, calls } = harness();
  await server.handleRequest("initialize", { protocolVersion: 2 });
  assert.deepEqual(
    await server.handleRequest("_buzz/session/dispose", {
      sessionId: "ses_test",
      forget: true,
    }),
    { disposed: true, forgotten: true },
  );
  assert.deepEqual(calls.dispose, [{ sessionId: "ses_test", forget: true }]);
  await assert.rejects(
    () => server.handleRequest("_session/dispose", { sessionId: "ses_test" }),
    /Method not found/,
  );
});

test("conversation reset commit is bounded and does not require a live session id", async () => {
  const { server, calls } = harness();
  await server.handleRequest("initialize", { protocolVersion: 2 });
  assert.deepEqual(
    await server.handleRequest("_buzz/conversation/reset", {
      conversationId: "channel:root",
      resetToken: "signed-reset-1",
    }),
    { committed: true, alreadyCommitted: false },
  );
  assert.deepEqual(calls.reset, [
    { conversationId: "channel:root", resetToken: "signed-reset-1" },
  ]);
  await assert.rejects(
    () =>
      server.handleRequest("_buzz/conversation/reset", {
        conversationId: "x".repeat(513),
        resetToken: "signed-reset-2",
      }),
    /must not exceed 512/,
  );
});

test("usage notifications match Buzz UsageTracker and keep cached tokens a subset", () => {
  const { server, writer } = harness();
  server.usageUpdate(
    "ses_test",
    {
      contextTokens: 140_000,
      accumulatedInputTokens: 10_000,
      accumulatedOutputTokens: 2_000,
      accumulatedCachedInputTokens: 99_000,
      accumulatedCost: 1.25,
      model: "provider/model",
    },
    150_000,
  );
  assert.deepEqual(writer.messages[0], {
    jsonrpc: "2.0",
    method: "_goose/unstable/session/update",
    params: {
      sessionId: "ses_test",
      update: {
        sessionUpdate: "usage_update",
        used: 140_000,
        contextLimit: 150_000,
        accumulatedInputTokens: 10_000,
        accumulatedOutputTokens: 2_000,
        accumulatedCachedInputTokens: 10_000,
        accumulatedCost: 1.25,
        model: "provider/model",
      },
    },
  });
});

test("usage notifications omit unknown current context instead of reporting a false zero", () => {
  const { server, writer } = harness();
  server.usageUpdate(
    "ses_test",
    {
      contextTokens: null,
      accumulatedInputTokens: 10,
      accumulatedOutputTokens: 2,
      accumulatedCachedInputTokens: 3,
      accumulatedCost: null,
      model: null,
    },
    150_000,
  );
  assert.equal("used" in writer.messages[0].params.update, false);
});

test("session update sanitization has one cycle-safe global traversal budget", () => {
  const { server, writer } = harness();
  const broad = {};
  broad.self = broad;
  for (let index = 0; index < 5_000; index += 1) {
    broad[`field-${index}`] = { shared: broad };
  }
  server.sessionUpdate("ses_test", broad);
  const update = writer.messages[0].params.update;
  assert.equal(update.self, "[circular]");
  assert.ok(Object.keys(update).length <= 100);
  assert.ok(Buffer.byteLength(JSON.stringify(update)) < 256 * 1_024);
});

test("unmapped compaction and context events are suppressed under the negotiated schema-v2 contract", async () => {
  const { server, writer, calls } = harness();
  const initialized = await server.handleRequest("initialize", {
    protocolVersion: 2,
  });
  assert.equal(initialized._meta.buzz.sessionEvents.schemaVersion, 2);

  server.buzzSessionEvent(
    "ses_unmapped",
    {
      type: "compaction_completed",
      compactionId: "9ba32f72-e8ce-4195-96a2-7b472198bb7e",
      timestamp: "2026-08-02T00:00:00.000Z",
      message: "Context compacted",
      piSessionId: "pi_test",
      reason: "threshold",
      beforeTokens: 140_000,
      afterTokens: 30_000,
      limitTokens: 150_000,
      effectiveLimitTokens: 150_000,
      compactionThresholdTokens: 133_616,
      willRetry: false,
      fromExtension: false,
    },
    "9ba32f72-e8ce-4195-96a2-7b472198bb7e",
  );
  const contextEvent = {
    type: "context_status",
    timestamp: "2026-08-02T00:00:01.000Z",
    message: "Context is at 50%.",
    piSessionId: "pi_test",
    usedTokens: 75_000,
    remainingTokens: 75_000,
    percent: 50,
    limitTokens: 150_000,
    effectiveLimitTokens: 150_000,
    compactionThresholdTokens: 133_616,
    autoCompaction: true,
    compacting: false,
    model: "provider/model",
  };
  server.buzzSessionEvent("ses_unmapped", contextEvent);
  assert.throws(
    () =>
      server.buzzSessionEvent(
        "ses_unmapped",
        contextEvent,
        "9BA32F72-E8CE-4195-96A2-7B472198BB7E",
      ),
    /deliveryId must be a lowercase UUID/,
  );

  assert.deepEqual(
    writer.messages.filter(
      (message) => message.method === "_buzz/session/event",
    ),
    [],
  );
  assert.deepEqual(calls.persistedEvents, []);
});

test("session/new fences early Pi lifecycle handoffs until the durable route commits", async () => {
  const lifecycleGeneration = "d".repeat(64);
  const deliveryId = "9ba32f72-e8ce-4195-96a2-7b472198bb7e";
  const conversationId = "channel:early-lifecycle";
  const timeline = [];
  const pendingEvents = [];
  let mappingCommitted = false;
  let allowMappingCommit;
  const mappingCommitGate = new Promise((resolve) => {
    allowMappingCommit = resolve;
  });
  let signalEarlyEvents;
  const earlyEventsEmitted = new Promise((resolve) => {
    signalEarlyEvents = resolve;
  });
  let childDeliveryOutcome;
  const deliveredEvent = {
    type: "context_status",
    timestamp: "2026-08-02T00:00:00.000Z",
    message: "Pi emitted while its durable route was still committing.",
    piSessionId: "pi-early-lifecycle",
    usedTokens: 75_000,
    remainingTokens: 75_000,
    percent: 50,
    limitTokens: 150_000,
    effectiveLimitTokens: 150_000,
    compactionThresholdTokens: 133_616,
    autoCompaction: true,
    compacting: false,
    model: "provider/model",
  };
  const syntheticEvent = {
    ...deliveredEvent,
    timestamp: "2026-08-02T00:00:01.000Z",
    message: "The adapter also emitted an early synthetic lifecycle event.",
  };
  const conversations = {
    async initialize() {},
    async resolve(resolvedConversationId, _resetToken, cwd, create) {
      assert.equal(resolvedConversationId, conversationId);
      const created = await create(undefined, lifecycleGeneration);
      timeline.push("factory-returned");
      await mappingCommitGate;
      mappingCommitted = true;
      timeline.push("mapping-committed");
      return {
        mapping: {
          conversationId: resolvedConversationId,
          cwd,
          ...created,
          lifecycleGeneration,
        },
        lifecycleGeneration,
        resumed: false,
        skipRelayHistory: false,
        async refresh() {
          return true;
        },
        async forget() {
          return undefined;
        },
        async release() {},
      };
    },
    async enqueueSessionEvent(
      persistedConversationId,
      eventId,
      event,
      expectedLifecycleGeneration,
    ) {
      assert.equal(mappingCommitted, true);
      assert.equal(persistedConversationId, conversationId);
      assert.equal(expectedLifecycleGeneration, lifecycleGeneration);
      timeline.push(
        eventId === deliveryId ? "parent-outbox" : "synthetic-parent-outbox",
      );
      pendingEvents.push({
        conversationId: persistedConversationId,
        eventId,
        lifecycleGeneration,
        event,
        createdAt: "2026-08-02T00:00:02.000Z",
      });
      return true;
    },
    async listPendingSessionEvents(persistedConversationId) {
      assert.equal(persistedConversationId, conversationId);
      return pendingEvents;
    },
    async acknowledgeSessionEvent() {},
    async deleteSessionFile() {},
    async prune() {
      return 0;
    },
  };
  let server;
  const forwardingSink = {
    sessionUpdate(...args) {
      return server.sessionUpdate(...args);
    },
    buzzSessionEvent(...args) {
      return server.buzzSessionEvent(...args);
    },
    usageUpdate(...args) {
      return server.usageUpdate(...args);
    },
  };
  const registry = new SessionRegistry(
    {
      async create(options) {
        const childDelivery = options.eventSink.buzzSessionEvent(
          options.acpSessionId,
          deliveredEvent,
          deliveryId,
        );
        childDeliveryOutcome = Promise.resolve(childDelivery).then(
          () => {
            timeline.push("child-ack");
            return "acked";
          },
          () => "rejected",
        );
        options.eventSink.buzzSessionEvent(
          options.acpSessionId,
          syntheticEvent,
        );
        timeline.push("child-emitted");
        signalEarlyEvents();
        return fakeHandle({
          piSessionId: "pi-early-lifecycle",
          sessionFile: "/tmp/pi-early-lifecycle.jsonl",
        });
      },
    },
    conversations,
    testConfig(),
    forwardingSink,
    silentLogger,
  );
  const input = new PassThrough();
  const writer = new MemoryWriter();
  let signalResponse;
  const responseWritten = new Promise((resolve) => {
    signalResponse = resolve;
  });
  let signalLifecycleNotification;
  const lifecycleNotificationWritten = new Promise((resolve) => {
    signalLifecycleNotification = resolve;
  });
  const write = writer.write.bind(writer);
  writer.write = (message) => {
    write(message);
    if (message.id === 42) {
      timeline.push("session-new-response");
      signalResponse(message);
    }
    if (message.method === "_buzz/session/event") {
      timeline.push("lifecycle-notification");
      signalLifecycleNotification(message);
    }
  };
  server = new AcpServer(input, writer, registry, testConfig(), silentLogger);
  await server.handleRequest("initialize", { protocolVersion: 2 });
  const running = server.run();
  input.write(
    `${JSON.stringify({
      jsonrpc: "2.0",
      id: 42,
      method: "session/new",
      params: {
        cwd: "/tmp",
        _meta: { buzz: { conversationId } },
      },
    })}\n`,
  );

  await earlyEventsEmitted;
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(mappingCommitted, false);
  assert.deepEqual(pendingEvents, []);
  assert.equal(timeline.includes("child-ack"), false);
  assert.equal(
    writer.messages.some((message) => message.method === "_buzz/session/event"),
    false,
  );

  allowMappingCommit();
  const response = await responseWritten;
  assert.equal(response.error, undefined);
  assert.equal(response.result.sessionId.startsWith("ses_"), true);
  assert.equal(await childDeliveryOutcome, "acked");
  assert.equal(pendingEvents.length, 2);
  assert.ok(
    timeline.indexOf("mapping-committed") < timeline.indexOf("parent-outbox"),
  );
  assert.ok(timeline.indexOf("parent-outbox") < timeline.indexOf("child-ack"));
  assert.equal(timeline.includes("lifecycle-notification"), false);

  await lifecycleNotificationWritten;
  await new Promise((resolve) => setImmediate(resolve));
  const notices = writer.messages.filter(
    (message) => message.method === "_buzz/session/event",
  );
  assert.equal(notices.length, 2);
  assert.ok(
    timeline.indexOf("session-new-response") <
      timeline.indexOf("lifecycle-notification"),
  );
  assert.deepEqual(
    new Set(notices.map((notice) => notice.params.event.message)),
    new Set([deliveredEvent.message, syntheticEvent.message]),
  );

  input.end();
  await running;
});

test("failed mapped creation rejects an early Pi lifecycle handoff without ACK or publication", async () => {
  const lifecycleGeneration = "e".repeat(64);
  const deliveryId = "73e58446-5e28-4a66-90fa-0fcba8cbf7da";
  const event = {
    type: "context_status",
    timestamp: "2026-08-02T00:00:00.000Z",
    message: "This event must remain child-owned when mapping commit fails.",
    piSessionId: "pi-failed-create",
    usedTokens: 1,
    remainingTokens: 149_999,
    percent: 0,
    limitTokens: 150_000,
    effectiveLimitTokens: 150_000,
    compactionThresholdTokens: 133_616,
    autoCompaction: true,
    compacting: false,
    model: "provider/model",
  };
  let persisted = false;
  let createdSessionId;
  let childOutcome;
  let server;
  const forwardingSink = {
    sessionUpdate(...args) {
      return server.sessionUpdate(...args);
    },
    buzzSessionEvent(...args) {
      return server.buzzSessionEvent(...args);
    },
    usageUpdate(...args) {
      return server.usageUpdate(...args);
    },
  };
  const conversations = {
    async initialize() {},
    async resolve(_conversationId, _resetToken, _cwd, create) {
      await create(undefined, lifecycleGeneration);
      throw new Error("manifest commit failed");
    },
    async enqueueSessionEvent() {
      persisted = true;
      return true;
    },
    async listPendingSessionEvents() {
      return [];
    },
    async acknowledgeSessionEvent() {},
    async deleteSessionFile() {},
    async prune() {
      return 0;
    },
  };
  const registry = new SessionRegistry(
    {
      async create(options) {
        createdSessionId = options.acpSessionId;
        const handoff = options.eventSink.buzzSessionEvent(
          options.acpSessionId,
          event,
          deliveryId,
        );
        childOutcome = Promise.resolve(handoff).then(
          () => "acked",
          () => "rejected",
        );
        return fakeHandle({
          piSessionId: "pi-failed-create",
          sessionFile: "/tmp/pi-failed-create.jsonl",
        });
      },
    },
    conversations,
    testConfig(),
    forwardingSink,
    silentLogger,
  );
  const writer = new MemoryWriter();
  server = new AcpServer(
    Readable.from([]),
    writer,
    registry,
    testConfig(),
    silentLogger,
  );
  await registry.start();
  await server.handleRequest("initialize", { protocolVersion: 2 });
  await assert.rejects(
    () =>
      server.handleRequest("session/new", {
        cwd: "/tmp",
        _meta: { buzz: { conversationId: "channel:failed-create" } },
      }),
    /manifest commit failed/,
  );
  assert.equal(await childOutcome, "rejected");
  assert.equal(persisted, false);
  assert.equal(
    writer.messages.some((message) => message.method === "_buzz/session/event"),
    false,
  );
  assert.equal(
    registry.conversationIdentityForSession(createdSessionId),
    undefined,
  );
  await server.shutdown();
});

test("mapped lifecycle events are durably persisted before schema-v2 publication and ACKed idempotently", async () => {
  const { server, writer, calls } = harness();
  await server.handleRequest("initialize", { protocolVersion: 2 });
  await server.handleRequest("session/new", {
    cwd: "/tmp/project",
    _meta: { buzz: { conversationId: "channel:root" } },
  });
  const event = {
    type: "context_status",
    timestamp: "2026-08-02T00:00:00.000Z",
    message: "Context is at 50%.",
    piSessionId: "pi_test",
    usedTokens: 75_000,
    remainingTokens: 75_000,
    percent: 50,
    limitTokens: 150_000,
    effectiveLimitTokens: 150_000,
    compactionThresholdTokens: 133_616,
    autoCompaction: true,
    compacting: false,
    model: "provider/model",
  };

  server.buzzSessionEvent("ses_test", event);
  await server.handleRequest("session/prompt", {
    sessionId: "ses_test",
    prompt: [{ type: "text", text: "continue" }],
  });

  assert.equal(calls.persistedEvents.length, 1);
  const persisted = calls.persistedEvents[0];
  assert.equal(persisted.conversationId, "channel:root");
  assert.deepEqual(persisted.event, event);
  assert.match(persisted.eventId, /^[0-9a-f-]{36}$/u);
  const notice = writer.messages.find(
    (message) =>
      message.method === "_buzz/session/event" &&
      message.params?.schemaVersion === 2,
  );
  assert.deepEqual(notice.params, {
    schemaVersion: 2,
    sessionId: "ses_test",
    conversationId: "channel:root",
    eventId: persisted.eventId,
    event,
  });
  assert.ok(
    calls.timeline.findIndex(
      (entry) =>
        entry.type === "persist" && entry.eventId === persisted.eventId,
    ) <
      calls.timeline.findIndex(
        (entry) =>
          entry.type === "write" && entry.eventId === persisted.eventId,
      ),
    "the durable write must complete before the notification is published",
  );

  for (let attempt = 0; attempt < 2; attempt += 1) {
    assert.deepEqual(
      await server.handleRequest("_buzz/session/event_ack", {
        conversationId: "channel:root",
        eventId: persisted.eventId,
      }),
      { acknowledged: true },
    );
  }
  assert.deepEqual(calls.acknowledgements, [
    { conversationId: "channel:root", eventId: persisted.eventId },
    { conversationId: "channel:root", eventId: persisted.eventId },
  ]);
});

test("session/new replays one lifecycle epoch across non-reset Pi replacements", async () => {
  const currentEventId = "9ba32f72-e8ce-4195-96a2-7b472198bb7e";
  const staleEventId = "b8ba08e4-65f5-4aed-9406-6c67fe8375db";
  const common = {
    type: "context_status",
    timestamp: "2026-08-02T00:00:00.000Z",
    message: "Context status.",
    usedTokens: 75_000,
    remainingTokens: 75_000,
    percent: 50,
    limitTokens: 150_000,
    effectiveLimitTokens: 150_000,
    compactionThresholdTokens: 133_616,
    autoCompaction: true,
    compacting: false,
    model: "provider/model",
  };
  const { server, writer, calls } = harness(
    {},
    {
      pendingEvents: [
        {
          conversationId: "channel:root",
          eventId: staleEventId,
          lifecycleGeneration: CURRENT_LIFECYCLE_GENERATION,
          event: { ...common, piSessionId: "pi_superseded" },
          createdAt: "2026-08-02T00:00:00.000Z",
        },
        {
          conversationId: "channel:root",
          eventId: currentEventId,
          lifecycleGeneration: CURRENT_LIFECYCLE_GENERATION,
          event: { ...common, piSessionId: "pi_test" },
          createdAt: "2026-08-02T00:00:01.000Z",
        },
      ],
    },
  );
  await server.handleRequest("initialize", { protocolVersion: 2 });
  await server.handleRequest("session/new", {
    cwd: "/tmp/project",
    _meta: { buzz: { conversationId: "channel:root" } },
  });
  await new Promise((resolve) => setImmediate(resolve));

  const notices = writer.messages.filter(
    (message) => message.method === "_buzz/session/event",
  );
  assert.deepEqual(
    notices.map((message) => message.params.eventId),
    [staleEventId, currentEventId],
  );
  assert.equal(notices[0].params.sessionId, "ses_test");
  assert.deepEqual(calls.acknowledgements, []);
});

test("session/new suppresses and ACKs only an older authenticated-reset lifecycle epoch", async () => {
  const currentEventId = "9ba32f72-e8ce-4195-96a2-7b472198bb7e";
  const resetSupersededEventId = "b8ba08e4-65f5-4aed-9406-6c67fe8375db";
  const common = {
    type: "context_status",
    timestamp: "2026-08-02T00:00:00.000Z",
    message: "Context status.",
    usedTokens: 75_000,
    remainingTokens: 75_000,
    percent: 50,
    limitTokens: 150_000,
    effectiveLimitTokens: 150_000,
    compactionThresholdTokens: 133_616,
    autoCompaction: true,
    compacting: false,
    model: "provider/model",
  };
  const { server, writer, calls } = harness(
    {},
    {
      pendingEvents: [
        {
          conversationId: "channel:root",
          eventId: resetSupersededEventId,
          lifecycleGeneration: "b".repeat(64),
          event: { ...common, piSessionId: "pi_before_reset" },
          createdAt: "2026-08-02T00:00:00.000Z",
        },
        {
          conversationId: "channel:root",
          eventId: currentEventId,
          lifecycleGeneration: CURRENT_LIFECYCLE_GENERATION,
          event: { ...common, piSessionId: "pi_after_reset" },
          createdAt: "2026-08-02T00:00:01.000Z",
        },
      ],
    },
  );
  await server.handleRequest("initialize", { protocolVersion: 2 });
  await server.handleRequest("session/new", {
    cwd: "/tmp/project",
    _meta: { buzz: { conversationId: "channel:root" } },
  });
  await new Promise((resolve) => setImmediate(resolve));

  const notices = writer.messages.filter(
    (message) => message.method === "_buzz/session/event",
  );
  assert.deepEqual(
    notices.map((message) => message.params.eventId),
    [currentEventId],
  );
  assert.deepEqual(calls.acknowledgements, [
    {
      conversationId: "channel:root",
      eventId: resetSupersededEventId,
    },
  ]);
});

test("lifecycle persistence failure poisons only the live session and churn remains bounded", async () => {
  const persistenceError = new Error("lifecycle outbox unavailable");
  const { server, writer } = harness(
    {},
    {
      persistError: persistenceError,
      sessionIdFactory: (index) => `ses_churn_${index}`,
    },
  );
  await server.handleRequest("initialize", { protocolVersion: 2 });

  for (let index = 1; index <= 24; index += 1) {
    const conversationId = `channel:root:${index}`;
    const created = await server.handleRequest("session/new", {
      cwd: "/tmp/project",
      _meta: { buzz: { conversationId } },
    });
    server.buzzSessionEvent(created.sessionId, {
      type: "context_status",
      timestamp: "2026-08-02T00:00:00.000Z",
      message: "Context status.",
      piSessionId: "pi_test",
      usedTokens: 1,
      remainingTokens: 149_999,
      percent: 0,
      limitTokens: 150_000,
      effectiveLimitTokens: 150_000,
      compactionThresholdTokens: 133_616,
      autoCompaction: true,
      compacting: false,
      model: "provider/model",
    });
    await assert.rejects(
      () =>
        server.handleRequest("session/prompt", {
          sessionId: created.sessionId,
          prompt: [{ type: "text", text: "continue" }],
        }),
      /lifecycle outbox unavailable/,
    );
    await assert.rejects(
      () =>
        server.handleRequest("session/prompt", {
          sessionId: created.sessionId,
          prompt: [{ type: "text", text: "retry" }],
        }),
      /lifecycle outbox unavailable/,
      "a live session remains fail-closed until its recovery boundary",
    );

    if (index % 2 === 0) {
      await server.handleRequest("_buzz/conversation/reset", {
        conversationId,
        resetToken: `reset-${index}`,
      });
    } else {
      await server.handleRequest("_buzz/session/dispose", {
        sessionId: created.sessionId,
      });
    }
    assert.equal(server.lifecycleFailures.size, 0);
  }
  await server.shutdown();
  assert.equal(
    writer.messages.some(
      (message) =>
        message.method === "_buzz/session/event" &&
        message.params?.schemaVersion === 2,
    ),
    false,
    "a lifecycle event that missed durable persistence must never be emitted",
  );
  assert.equal(server.lifecycleFailures.size, 0);
  assert.equal(server.publishedLifecycleEventIds.size, 0);
});

test("strict context refusal has a stable terminal ACP error code", async () => {
  const { server, writer } = harness({
    async prompt() {
      throw new Error(
        "BUZZ_CONTEXT_LIMIT: final provider context exceeds 150000 tokens",
      );
    },
  });
  await server.handleRequest("initialize", { protocolVersion: 2 });
  await server.dispatch({
    jsonrpc: "2.0",
    id: 42,
    method: "session/prompt",
    params: {
      sessionId: "ses_test",
      prompt: [{ type: "text", text: "continue" }],
    },
  });
  assert.deepEqual(writer.messages.at(-1).error, {
    code: -32042,
    message: "BUZZ_CONTEXT_LIMIT: final provider context exceeds 150000 tokens",
    data: { kind: "context_limit", retryable: false },
  });
});

test("transcript quota refusal is terminal and points Buzz users to /new", async () => {
  const { server, writer } = harness({
    async prompt() {
      throw new Error(
        "BUZZ_SESSION_STORAGE_LIMIT: this Pi thread transcript reached its 64.0 MiB storage ceiling; use /new to start a fresh session",
      );
    },
  });
  await server.handleRequest("initialize", { protocolVersion: 2 });
  await server.dispatch({
    jsonrpc: "2.0",
    id: 43,
    method: "session/prompt",
    params: {
      sessionId: "ses_test",
      prompt: [{ type: "text", text: "continue" }],
    },
  });
  assert.deepEqual(writer.messages.at(-1).error, {
    code: -32044,
    message:
      "BUZZ_SESSION_STORAGE_LIMIT: this Pi thread transcript reached its 64.0 MiB storage ceiling; use /new to start a fresh session",
    data: {
      kind: "session_storage_limit",
      retryable: false,
      recovery: "/new",
    },
  });
});

test("a poisoned Pi generation has a stable retryable ACP error code", async () => {
  const { server, writer } = harness({
    async prompt() {
      throw new Error(
        "BUZZ_PI_SESSION_INVALIDATED: Pi resource reload failed; create a fresh session",
      );
    },
  });
  await server.handleRequest("initialize", { protocolVersion: 2 });
  await server.dispatch({
    jsonrpc: "2.0",
    id: 44,
    method: "session/prompt",
    params: {
      sessionId: "ses_test",
      prompt: [{ type: "text", text: "continue" }],
    },
  });
  assert.deepEqual(writer.messages.at(-1).error, {
    code: -32045,
    message:
      "BUZZ_PI_SESSION_INVALIDATED: Pi resource reload failed; create a fresh session",
    data: { kind: "session_invalidated", retryable: true },
  });
});

test("stdout saturation proactively aborts a never-ending active prompt", async () => {
  const input = new PassThrough();
  let releasePrompt;
  let aborts = 0;
  const handle = fakeHandle({
    async prompt() {
      return new Promise((resolve) => {
        releasePrompt = resolve;
      });
    },
    async abort() {
      aborts += 1;
      releasePrompt?.("cancelled");
    },
  });
  let registryClosing = false;
  const registry = {
    async start() {},
    hasSession() {
      return !registryClosing;
    },
    get() {
      return handle;
    },
    async shutdown() {
      if (registryClosing) return;
      registryClosing = true;
      if (handle.isBusy) await handle.abort();
      await handle.dispose();
    },
  };
  let server;
  const writer = new NdjsonWriter(() => new Promise(() => {}), silentLogger, {
    maxQueuedMessages: 2,
    maxQueuedBytes: 64 * 1_024,
    onFatal(error) {
      input.destroy(error);
      void server.shutdown().catch(() => {});
    },
  });
  server = new AcpServer(input, writer, registry, testConfig(), silentLogger);
  const running = server.run();
  input.write(
    `${[
      { jsonrpc: "2.0", id: 1, method: "initialize", params: {} },
      {
        jsonrpc: "2.0",
        id: 2,
        method: "session/prompt",
        params: {
          sessionId: "ses_test",
          prompt: [{ type: "text", text: "wait forever" }],
        },
      },
      { jsonrpc: "2.0", id: 3, method: "unknown/one", params: {} },
      { jsonrpc: "2.0", id: 4, method: "unknown/two", params: {} },
    ]
      .map((message) => JSON.stringify(message))
      .join("\n")}\n`,
  );

  const outcome = await Promise.race([
    running.then(
      () => "finished",
      () => "failed",
    ),
    new Promise((resolve) => setTimeout(() => resolve("timed-out"), 500)),
  ]);
  assert.notEqual(outcome, "timed-out");
  assert.equal(aborts, 1);
  assert.equal(handle.disposed, true);
});

test("active request flood is bounded while cancellation keeps reserved capacity", async () => {
  const input = new PassThrough();
  const writer = new MemoryWriter();
  let releasePrompt;
  let aborts = 0;
  const handle = fakeHandle({
    async prompt() {
      return new Promise((resolve) => {
        releasePrompt = resolve;
      });
    },
    async abort() {
      aborts += 1;
      releasePrompt?.("cancelled");
    },
  });
  let registryClosing = false;
  const registry = {
    async start() {},
    hasSession() {
      return !registryClosing;
    },
    get() {
      return handle;
    },
    async shutdown() {
      if (registryClosing) return;
      registryClosing = true;
      if (handle.isBusy) await handle.abort();
      await handle.dispose();
    },
  };
  const server = new AcpServer(
    input,
    writer,
    registry,
    testConfig({ maxActiveRequests: 1 }),
    silentLogger,
  );
  const running = server.run();
  input.write(
    `${JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize", params: {} })}\n`,
  );
  while (!writer.messages.some((message) => message.id === 1)) {
    await new Promise((resolve) => setImmediate(resolve));
  }
  input.write(
    `${JSON.stringify({
      jsonrpc: "2.0",
      id: 2,
      method: "session/prompt",
      params: {
        sessionId: "ses_test",
        prompt: [{ type: "text", text: "hold" }],
      },
    })}\n`,
  );
  while (!handle.isBusy) {
    await new Promise((resolve) => setImmediate(resolve));
  }
  input.end(
    `${[
      ...Array.from({ length: 5 }, (_, index) => ({
        jsonrpc: "2.0",
        id: 3 + index,
        method: `flood/${index}`,
        params: {},
      })),
      {
        jsonrpc: "2.0",
        id: 8,
        method: "session/cancel",
        params: { sessionId: "ses_test" },
      },
    ]
      .map((message) => JSON.stringify(message))
      .join("\n")}\n`,
  );
  await running;

  const overloads = writer.messages.filter(
    (message) => message.error?.code === -32043,
  );
  assert.deepEqual(
    overloads.map((message) => message.id),
    [3, 4, 5, 6, 7],
  );
  assert.ok(
    writer.messages.some(
      (message) => message.id === 8 && message.result === null,
    ),
  );
  assert.equal(aborts, 1);
});

test("durable mapped Buzz lifecycle v2 fixture is emitted without schema drift", async () => {
  const fixture = JSON.parse(
    await readFile(
      new URL("./fixtures/buzz-session-event-v2.json", import.meta.url),
      "utf8",
    ),
  );
  const { server, writer } = harness(
    { piSessionId: fixture.event.piSessionId },
    { sessionIdFactory: () => fixture.sessionId },
  );
  await server.handleRequest("initialize", { protocolVersion: 2 });
  await server.handleRequest("session/new", {
    cwd: "/tmp/project",
    _meta: { buzz: { conversationId: fixture.conversationId } },
  });
  await server.buzzSessionEvent(
    fixture.sessionId,
    fixture.event,
    fixture.eventId,
  );
  const lifecycle = writer.messages.find(
    (message) => message.method === "_buzz/session/event",
  );
  assert.deepEqual(lifecycle.params, fixture);
});
