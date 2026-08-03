import assert from "node:assert/strict";
import {
  access,
  mkdir,
  mkdtemp,
  realpath,
  symlink,
  unlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { SessionRegistry } from "../dist/index.js";
import { fakeHandle, silentLogger, testConfig } from "./helpers.mjs";

const sink = {
  sessionUpdate() {},
  buzzSessionEvent() {},
  usageUpdate() {},
};

function fakeConversationStore() {
  const mappings = new Map();
  const resetTombstones = new Map();
  let lifecycleGenerationIndex = 0;
  return {
    async initialize() {},
    async resolve(conversationId, resetToken, cwd, create) {
      const existing = mappings.get(conversationId);
      const tombstone = resetTombstones.get(conversationId);
      const forceFresh =
        tombstone !== undefined ||
        (resetToken && resetToken !== existing?.resetToken);
      const resetBoundary = forceFresh;
      const lifecycleGeneration = resetBoundary
        ? `${(++lifecycleGenerationIndex).toString(16).padStart(64, "0")}`
        : (existing?.lifecycleGeneration ?? "0".repeat(64));
      const created = await create(
        forceFresh ? undefined : existing?.sessionFile,
        lifecycleGeneration,
      );
      const mapping = {
        conversationId,
        resetToken: resetToken ?? tombstone?.resetToken,
        sessionFile: created.sessionFile,
        piSessionId: created.piSessionId,
        lifecycleGeneration,
        cwd,
      };
      mappings.set(conversationId, mapping);
      resetTombstones.delete(conversationId);
      return {
        mapping,
        lifecycleGeneration: mapping.lifecycleGeneration,
        resumed: existing !== undefined && !forceFresh,
        ...(forceFresh && existing
          ? { retiredSessionFile: existing.sessionFile }
          : {}),
        ...(forceFresh && (existing || tombstone)
          ? {
              previousPiSessionId:
                existing?.piSessionId ?? tombstone?.previousPiSessionId,
            }
          : {}),
        skipRelayHistory: tombstone !== undefined || Boolean(forceFresh),
        async refresh() {
          return (
            mappings.get(conversationId)?.piSessionId === mapping.piSessionId
          );
        },
        async forget() {
          const current = mappings.get(conversationId);
          if (current?.piSessionId !== mapping.piSessionId) return undefined;
          mappings.delete(conversationId);
          resetTombstones.set(conversationId, {
            previousPiSessionId: mapping.piSessionId,
          });
          return mapping.sessionFile;
        },
        async release() {},
      };
    },
    async touch() {},
    async forget(conversationId, expectedPiSessionId) {
      const mapping = mappings.get(conversationId);
      if (mapping?.piSessionId !== expectedPiSessionId) return undefined;
      mappings.delete(conversationId);
      resetTombstones.set(conversationId, {
        previousPiSessionId: mapping.piSessionId,
      });
      return mapping.sessionFile;
    },
    async commitReset(conversationId, resetToken) {
      const mapping = mappings.get(conversationId);
      const tombstone = resetTombstones.get(conversationId);
      if (
        mapping?.resetToken === resetToken ||
        tombstone?.resetToken === resetToken
      ) {
        return {
          alreadyCommitted: true,
          disposeLiveSession: tombstone?.resetToken === resetToken,
        };
      }
      mappings.delete(conversationId);
      resetTombstones.set(conversationId, {
        resetToken,
        previousPiSessionId:
          mapping?.piSessionId ?? tombstone?.previousPiSessionId,
      });
      return {
        alreadyCommitted: false,
        disposeLiveSession: true,
        ...(mapping ? { retiredSessionFile: mapping.sessionFile } : {}),
      };
    },
    async deleteSessionFile() {},
    async prune() {
      return 0;
    },
  };
}

test("concurrent duplicate session/new for one conversation creates exactly one Pi session", async () => {
  let creates = 0;
  const factory = {
    async create() {
      creates++;
      await new Promise((resolve) => setTimeout(resolve, 10));
      return fakeHandle({
        piSessionId: "pi-shared",
        sessionFile: "/tmp/pi-shared.jsonl",
      });
    },
  };
  const registry = new SessionRegistry(
    factory,
    fakeConversationStore(),
    testConfig(),
    sink,
    silentLogger,
  );
  await registry.start();
  const [left, right] = await Promise.all([
    registry.create({
      cwd: "/tmp",
      conversationId: "thread",
      resetToken: "reset-1",
    }),
    registry.create({
      cwd: "/tmp",
      conversationId: "thread",
      resetToken: "reset-1",
    }),
  ]);
  assert.equal(creates, 1);
  assert.equal(left.sessionId, right.sessionId);
  await registry.shutdown();
});

test("a live conversation never follows a workspace symlink to a new target", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-registry-live-cwd-"));
  const workspaceA = join(root, "workspace-a");
  const workspaceB = join(root, "workspace-b");
  const alias = join(root, "workspace-current");
  await mkdir(workspaceA);
  await mkdir(workspaceB);
  await symlink(workspaceA, alias, "dir");
  const handles = [];
  const registry = new SessionRegistry(
    {
      async create(options) {
        const handle = fakeHandle({
          cwd: options.cwd,
          piSessionId: `pi-${handles.length}`,
          sessionFile: join(root, `pi-${handles.length}.jsonl`),
        });
        handles.push(handle);
        return handle;
      },
    },
    fakeConversationStore(),
    testConfig(),
    sink,
    silentLogger,
  );
  await registry.start();
  const first = await registry.create({ cwd: alias, conversationId: "thread" });
  await unlink(alias);
  await symlink(workspaceB, alias, "dir");
  const second = await registry.create({
    cwd: alias,
    conversationId: "thread",
  });

  assert.notEqual(first.sessionId, second.sessionId);
  assert.equal(first.handle.disposed, true);
  assert.equal(second.handle.cwd, await realpath(workspaceB));
  assert.equal(handles.length, 2);
  await registry.shutdown();
});

test("a pending conversation create rejects a workspace symlink target swap", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-registry-pending-cwd-"));
  const workspaceA = join(root, "workspace-a");
  const workspaceB = join(root, "workspace-b");
  const alias = join(root, "workspace-current");
  await mkdir(workspaceA);
  await mkdir(workspaceB);
  await symlink(workspaceA, alias, "dir");
  let releaseFactory;
  let enteredFactory;
  const entered = new Promise((resolve) => {
    enteredFactory = resolve;
  });
  const release = new Promise((resolve) => {
    releaseFactory = resolve;
  });
  const registry = new SessionRegistry(
    {
      async create(options) {
        enteredFactory();
        await release;
        return fakeHandle({
          cwd: options.cwd,
          piSessionId: "pi-pending",
          sessionFile: join(root, "pi-pending.jsonl"),
        });
      },
    },
    fakeConversationStore(),
    testConfig(),
    sink,
    silentLogger,
  );
  await registry.start();
  const first = registry.create({ cwd: alias, conversationId: "thread" });
  await entered;
  await unlink(alias);
  await symlink(workspaceB, alias, "dir");
  await assert.rejects(
    () => registry.create({ cwd: alias, conversationId: "thread" }),
    /different workspace is already creating this conversation/,
  );
  releaseFactory();
  await first;
  await registry.shutdown();
});

test("capacity promotion cannot evict a new session while retired-file deletion is pending", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-registry-promotion-"));
  const handles = [];
  let enterDeletion;
  let releaseDeletion;
  const deletionEntered = new Promise((resolve) => {
    enterDeletion = resolve;
  });
  const deletionRelease = new Promise((resolve) => {
    releaseDeletion = resolve;
  });
  const conversations = fakeConversationStore();
  conversations.deleteSessionFile = async () => {
    enterDeletion();
    await deletionRelease;
  };
  const registry = new SessionRegistry(
    {
      async create(options) {
        const handle = fakeHandle({
          cwd: options.cwd,
          piSessionId: `pi-promotion-${handles.length}`,
          sessionFile: join(root, `pi-promotion-${handles.length}.jsonl`),
        });
        handles.push(handle);
        return handle;
      },
    },
    conversations,
    testConfig({ maxSessions: 1 }),
    sink,
    silentLogger,
  );
  await registry.start();
  const first = await registry.create({
    cwd: root,
    conversationId: "thread",
    resetToken: "reset-1",
  });
  const replacementPromise = registry.create({
    cwd: root,
    conversationId: "thread",
    resetToken: "reset-2",
  });
  await deletionEntered;

  await assert.rejects(
    () =>
      registry.create({
        cwd: root,
        conversationId: "other-thread",
      }),
    /all slots are busy or initializing/,
  );
  releaseDeletion();
  const replacement = await replacementPromise;
  assert.equal(first.handle.disposed, true);
  assert.equal(replacement.handle.disposed, false);
  assert.equal(replacement.handle.isValid, true);
  assert.equal(registry.hasSession(replacement.sessionId), true);
  assert.equal(handles.length, 2);
  await registry.shutdown();
});

test("LRU eviction disposes the oldest idle handle but retains durable mapping", async () => {
  const handles = [];
  const factory = {
    async create() {
      const handle = fakeHandle({
        piSessionId: `pi-${handles.length}`,
        sessionFile: `/tmp/pi-${handles.length}.jsonl`,
      });
      handles.push(handle);
      return handle;
    },
  };
  let now = 1;
  const registry = new SessionRegistry(
    factory,
    fakeConversationStore(),
    testConfig({ maxSessions: 1 }),
    sink,
    silentLogger,
    () => now++,
  );
  await registry.start();
  await registry.create({ cwd: "/tmp", conversationId: "one" });
  await registry.create({ cwd: "/tmp", conversationId: "two" });
  assert.equal(handles[0].disposed, true);
  assert.equal(handles[1].disposed, false);
  assert.equal(registry.size, 1);
  await registry.shutdown();
});

test("conversation reset commit clears a cold LRU mapping before ACK and survives token-less recreation", async () => {
  const handles = [];
  const store = fakeConversationStore();
  const registry = new SessionRegistry(
    {
      async create() {
        const index = handles.length;
        const handle = fakeHandle({
          piSessionId: `pi-${index}`,
          sessionFile: `/tmp/pi-${index}.jsonl`,
        });
        handles.push(handle);
        return handle;
      },
    },
    store,
    testConfig({ maxSessions: 1 }),
    sink,
    silentLogger,
  );
  await registry.start();
  await registry.create({ cwd: "/tmp", conversationId: "thread" });
  await registry.create({ cwd: "/tmp", conversationId: "other" });
  assert.equal(handles[0].disposed, true, "thread must be cold before reset");

  assert.deepEqual(
    await registry.commitConversationReset("thread", "signed-reset-1"),
    { committed: true, alreadyCommitted: false },
  );
  const fresh = await registry.create({
    cwd: "/tmp",
    conversationId: "thread",
  });
  assert.equal(fresh.resumedConversation, false);
  assert.equal(fresh.skipRelayHistory, true);
  assert.equal(fresh.handle.piSessionId, "pi-2");

  assert.deepEqual(
    await registry.commitConversationReset("thread", "signed-reset-1"),
    { committed: true, alreadyCommitted: true },
  );
  assert.equal(
    fresh.handle.disposed,
    false,
    "an idempotent replay must not discard the already-fresh session",
  );
  await registry.shutdown();
});

test("dispose uses the resolved owner-bound forget closure and releases SDK resources", async () => {
  const store = fakeConversationStore();
  store.forget = async () => {
    throw new Error("unfenced store forget must not be called");
  };
  const handle = fakeHandle({
    piSessionId: "pi-one",
    sessionFile: "/tmp/pi-one.jsonl",
  });
  const registry = new SessionRegistry(
    {
      async create() {
        return handle;
      },
    },
    store,
    testConfig(),
    sink,
    silentLogger,
  );
  await registry.start();
  const created = await registry.create({
    cwd: "/tmp",
    conversationId: "thread",
  });
  assert.equal(await registry.disposeSession(created.sessionId, true), true);
  assert.equal(handle.disposed, true);
  assert.equal(registry.size, 0);
  await registry.shutdown();
});

test("normal disposal deletes an unmapped transcript but retains a mapped conversation", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-unmapped-dispose-"));
  const unmappedFile = join(root, "unmapped.jsonl");
  const mappedFile = join(root, "mapped.jsonl");
  await writeFile(unmappedFile, "unmapped");
  await writeFile(mappedFile, "mapped");
  const store = fakeConversationStore();
  store.deleteSessionFile = async (path) => unlink(path);
  let creates = 0;
  const registry = new SessionRegistry(
    {
      async create() {
        const mapped = creates++ > 0;
        return fakeHandle({
          piSessionId: mapped ? "pi-mapped" : "pi-unmapped",
          sessionFile: mapped ? mappedFile : unmappedFile,
        });
      },
    },
    store,
    testConfig(),
    sink,
    silentLogger,
  );
  await registry.start();
  const unmapped = await registry.create({ cwd: root });
  const mapped = await registry.create({
    cwd: root,
    conversationId: "thread",
  });

  assert.equal(await registry.disposeSession(unmapped.sessionId, false), true);
  await assert.rejects(() => access(unmappedFile), { code: "ENOENT" });
  assert.equal(await registry.disposeSession(mapped.sessionId, false), true);
  await access(mappedFile);
  await unlink(mappedFile);
  await registry.shutdown();
});

test("closing a mapped session retains its lifecycle route until buffered persistence is ACK-safe", async () => {
  const lifecycleGeneration = "c".repeat(64);
  const eventId = "9ba32f72-e8ce-4195-96a2-7b472198bb7e";
  const timeline = [];
  let signalDisposeStarted;
  const disposeStarted = new Promise((resolve) => {
    signalDisposeStarted = resolve;
  });
  let allowBufferedEvent;
  const bufferedEventGate = new Promise((resolve) => {
    allowBufferedEvent = resolve;
  });
  let signalBufferedEventDone;
  const bufferedEventDone = new Promise((resolve) => {
    signalBufferedEventDone = resolve;
  });
  let allowDisposeFinish;
  const disposeFinishGate = new Promise((resolve) => {
    allowDisposeFinish = resolve;
  });
  let registry;
  const lifecycleEvent = {
    type: "context_status",
    timestamp: "2026-08-02T00:00:00.000Z",
    message: "Buffered lifecycle status.",
    piSessionId: "pi-closing-route",
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
  const eventSink = {
    sessionUpdate() {},
    async buzzSessionEvent(sessionId, event, deliveryId) {
      const identity = registry.conversationIdentityForSession(sessionId);
      if (!identity) {
        timeline.push("suppressed");
        return;
      }
      const persisted = await registry.persistConversationSessionEvent(
        identity.conversationId,
        identity.lifecycleGeneration,
        deliveryId,
        event,
      );
      if (persisted) timeline.push("persisted");
    },
    usageUpdate() {},
  };
  const conversations = {
    async initialize() {},
    async resolve(conversationId, _resetToken, cwd, create) {
      const created = await create(undefined, lifecycleGeneration);
      return {
        mapping: {
          conversationId,
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
      conversationId,
      persistedEventId,
      event,
      expectedLifecycleGeneration,
    ) {
      assert.equal(conversationId, "thread");
      assert.equal(persistedEventId, eventId);
      assert.equal(event, lifecycleEvent);
      assert.equal(expectedLifecycleGeneration, lifecycleGeneration);
      timeline.push("parent-outbox");
      return true;
    },
    async deleteSessionFile() {},
    async prune() {
      return 0;
    },
  };
  registry = new SessionRegistry(
    {
      async create(options) {
        return fakeHandle({
          piSessionId: "pi-closing-route",
          sessionFile: "/tmp/pi-closing-route.jsonl",
          async dispose() {
            signalDisposeStarted();
            await bufferedEventGate;
            await eventSink.buzzSessionEvent(
              options.acpSessionId,
              lifecycleEvent,
              eventId,
            );
            timeline.push("ackLifecycle");
            signalBufferedEventDone();
            await disposeFinishGate;
          },
        });
      },
    },
    conversations,
    testConfig(),
    eventSink,
    silentLogger,
  );
  await registry.start();
  const created = await registry.create({
    cwd: "/tmp",
    conversationId: "thread",
  });
  const disposal = registry.disposeSession(created.sessionId, false);
  await disposeStarted;
  assert.equal(registry.hasSession(created.sessionId), false);
  assert.deepEqual(registry.conversationIdentityForSession(created.sessionId), {
    conversationId: "thread",
    lifecycleGeneration,
  });
  await assert.rejects(() => registry.get(created.sessionId), /closing/);

  allowBufferedEvent();
  await bufferedEventDone;
  assert.deepEqual(timeline, ["parent-outbox", "persisted", "ackLifecycle"]);
  assert.deepEqual(registry.conversationIdentityForSession(created.sessionId), {
    conversationId: "thread",
    lifecycleGeneration,
  });
  allowDisposeFinish();
  assert.equal(await disposal, true);
  assert.equal(
    registry.conversationIdentityForSession(created.sessionId),
    undefined,
  );
  await registry.shutdown();
});

test("TTL eviction deletes an unreachable unmapped transcript", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-unmapped-ttl-"));
  const sessionFile = join(root, "expired.jsonl");
  await writeFile(sessionFile, "expired");
  const store = fakeConversationStore();
  store.deleteSessionFile = async (path) => unlink(path);
  let now = 0;
  const handle = fakeHandle({ piSessionId: "pi-expired", sessionFile });
  const registry = new SessionRegistry(
    {
      async create() {
        return handle;
      },
    },
    store,
    testConfig({ sessionTtlMs: 100 }),
    sink,
    silentLogger,
    () => now,
  );
  await registry.start();
  await registry.create({ cwd: root });
  now = 101;

  assert.equal(await registry.sweepExpired(), 1);
  assert.equal(handle.disposed, true);
  await assert.rejects(() => access(sessionFile), { code: "ENOENT" });
  await registry.shutdown();
});

test("shutdown deletes unmapped transcripts and preserves mapped transcripts", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-unmapped-shutdown-"));
  const unmappedFile = join(root, "unmapped.jsonl");
  const mappedFile = join(root, "mapped.jsonl");
  await writeFile(unmappedFile, "unmapped");
  await writeFile(mappedFile, "mapped");
  const store = fakeConversationStore();
  store.deleteSessionFile = async (path) => unlink(path);
  const handles = [];
  const registry = new SessionRegistry(
    {
      async create() {
        const sessionFile = handles.length === 0 ? unmappedFile : mappedFile;
        const handle = fakeHandle({
          piSessionId: `pi-shutdown-${handles.length}`,
          sessionFile,
        });
        handles.push(handle);
        return handle;
      },
    },
    store,
    testConfig(),
    sink,
    silentLogger,
  );
  await registry.start();
  await registry.create({ cwd: root });
  await registry.create({ cwd: root, conversationId: "thread" });

  await registry.shutdown();
  assert.deepEqual(
    handles.map((handle) => handle.disposed),
    [true, true],
  );
  await assert.rejects(() => access(unmappedFile), { code: "ENOENT" });
  await access(mappedFile);
  await unlink(mappedFile);
});

test("runtime host invalidation deletes unmapped transcripts and preserves mapped transcripts", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-unmapped-invalidated-"));
  const unmappedFile = join(root, "unmapped.jsonl");
  const mappedFile = join(root, "mapped.jsonl");
  await writeFile(unmappedFile, "unmapped");
  await writeFile(mappedFile, "mapped");
  const store = fakeConversationStore();
  store.deleteSessionFile = async (path) => unlink(path);
  const handles = [];
  let invalidate;
  const factory = {
    setInvalidationHandler(handler) {
      invalidate = handler;
    },
    async create() {
      const sessionFile = handles.length === 0 ? unmappedFile : mappedFile;
      const handle = fakeHandle({
        piSessionId: `pi-invalidated-${handles.length}`,
        sessionFile,
      });
      handles.push(handle);
      return handle;
    },
  };
  const registry = new SessionRegistry(
    factory,
    store,
    testConfig(),
    sink,
    silentLogger,
  );
  await registry.start();
  const unmapped = await registry.create({ cwd: root });
  const mapped = await registry.create({
    cwd: root,
    conversationId: "thread",
  });

  await invalidate(
    [unmapped.sessionId, mapped.sessionId],
    new Error("runtime host crashed"),
  );
  assert.equal(registry.size, 0);
  assert.deepEqual(
    handles.map((handle) => handle.disposed),
    [true, true],
  );
  await assert.rejects(() => access(unmappedFile), { code: "ENOENT" });
  await access(mappedFile);
  await unlink(mappedFile);
  await registry.shutdown();
});

test("forget failure still disposes, releases the lease, and permits the next reset", async () => {
  let leased = false;
  let failForget = true;
  let lifecycleGenerationIndex = 0;
  let handleGeneration = 0;
  const mappings = new Map();
  const store = {
    async initialize() {},
    async resolve(conversationId, resetToken, cwd, create) {
      if (leased) throw new Error("active lease");
      const existing = mappings.get(conversationId);
      const forceFresh =
        resetToken !== undefined && resetToken !== existing?.resetToken;
      const lifecycleGeneration = forceFresh
        ? `${(++lifecycleGenerationIndex).toString(16).padStart(64, "0")}`
        : (existing?.lifecycleGeneration ?? "0".repeat(64));
      const created = await create(
        forceFresh ? undefined : existing?.sessionFile,
        lifecycleGeneration,
      );
      const mapping = {
        conversationId,
        resetToken,
        sessionFile: created.sessionFile,
        piSessionId: created.piSessionId,
        lifecycleGeneration,
        cwd,
      };
      mappings.set(conversationId, mapping);
      leased = true;
      return {
        mapping,
        lifecycleGeneration,
        resumed: existing !== undefined && !forceFresh,
        ...(forceFresh && existing
          ? { previousPiSessionId: existing.piSessionId }
          : {}),
        async forget() {
          return store.forget();
        },
        async release() {
          leased = false;
        },
      };
    },
    async touch() {},
    async forget() {
      if (failForget) {
        failForget = false;
        throw new Error("injected manifest failure");
      }
      return undefined;
    },
    async deleteSessionFile() {},
    async prune() {
      return 0;
    },
  };
  const handles = [];
  const registry = new SessionRegistry(
    {
      async create() {
        handleGeneration++;
        const handle = fakeHandle({
          piSessionId: `pi-${handleGeneration}`,
          sessionFile: `/tmp/pi-${handleGeneration}.jsonl`,
        });
        handles.push(handle);
        return handle;
      },
    },
    store,
    testConfig(),
    sink,
    silentLogger,
  );
  await registry.start();
  const first = await registry.create({
    cwd: "/tmp",
    conversationId: "thread",
  });
  await assert.rejects(
    () => registry.disposeSession(first.sessionId, true),
    /injected manifest failure/,
  );
  assert.equal(handles[0].disposed, true);
  assert.equal(registry.size, 0);
  assert.equal(leased, false, "failed forget must not strand a live-PID lease");

  const reset = await registry.create({
    cwd: "/tmp",
    conversationId: "thread",
    resetToken: "signed-reset-2",
  });
  assert.equal(reset.handle.piSessionId, "pi-2");
  await registry.shutdown();
});

test("concurrent unique creates never exceed the configured session capacity", async () => {
  let inFactory = 0;
  let peakFactory = 0;
  const registry = new SessionRegistry(
    {
      async create(options) {
        inFactory++;
        peakFactory = Math.max(peakFactory, inFactory);
        await new Promise((resolve) => setTimeout(resolve, 15));
        inFactory--;
        return fakeHandle({
          piSessionId: options.acpSessionId,
          sessionFile: `/tmp/${options.acpSessionId}.jsonl`,
        });
      },
    },
    fakeConversationStore(),
    testConfig({ maxSessions: 2 }),
    sink,
    silentLogger,
  );
  await registry.start();
  const results = await Promise.allSettled([
    registry.create({ cwd: "/tmp" }),
    registry.create({ cwd: "/tmp" }),
    registry.create({ cwd: "/tmp" }),
  ]);
  assert.equal(
    results.filter((result) => result.status === "fulfilled").length,
    2,
  );
  assert.equal(
    results.filter((result) => result.status === "rejected").length,
    1,
  );
  assert.equal(peakFactory, 2);
  assert.equal(registry.size, 2);
  await registry.shutdown();
});

test("a superseded conversation lease is fenced before its handle is returned", async () => {
  let current = true;
  let released = false;
  const handle = fakeHandle({
    piSessionId: "pi-fenced",
    sessionFile: "/tmp/pi-fenced.jsonl",
  });
  const registry = new SessionRegistry(
    {
      async create() {
        return handle;
      },
    },
    {
      async initialize() {},
      async resolve(conversationId, _resetToken, cwd, create) {
        const lifecycleGeneration = "1".repeat(64);
        const created = await create(undefined, lifecycleGeneration);
        return {
          mapping: {
            conversationId,
            cwd,
            ...created,
            lifecycleGeneration,
          },
          lifecycleGeneration,
          resumed: false,
          skipRelayHistory: false,
          async refresh() {
            return current;
          },
          async release() {
            released = true;
          },
        };
      },
      async prune() {
        return 0;
      },
    },
    testConfig(),
    sink,
    silentLogger,
  );
  await registry.start();
  const created = await registry.create({
    cwd: "/tmp",
    conversationId: "thread",
  });
  current = false;
  await assert.rejects(
    () => registry.get(created.sessionId),
    /lease was superseded/,
  );
  assert.equal(handle.disposed, true);
  assert.equal(released, true);
  assert.equal(registry.size, 0);
  await registry.shutdown();
});

test("a lease refresh exception disposes the session during background sweep", async () => {
  let released = false;
  const handle = fakeHandle({
    piSessionId: "pi-refresh-error",
    sessionFile: "/tmp/pi-refresh-error.jsonl",
  });
  const registry = new SessionRegistry(
    {
      async create() {
        return handle;
      },
    },
    {
      async initialize() {},
      async resolve(conversationId, _resetToken, cwd, create) {
        const lifecycleGeneration = "2".repeat(64);
        const created = await create(undefined, lifecycleGeneration);
        return {
          mapping: {
            conversationId,
            cwd,
            ...created,
            lifecycleGeneration,
          },
          lifecycleGeneration,
          resumed: false,
          skipRelayHistory: false,
          async refresh() {
            throw new Error("manifest unavailable");
          },
          async release() {
            released = true;
          },
        };
      },
      async prune() {
        return 0;
      },
    },
    testConfig(),
    sink,
    silentLogger,
  );
  await registry.start();
  await registry.create({ cwd: "/tmp", conversationId: "thread" });
  assert.equal(await registry.sweepExpired(), 0);
  assert.equal(handle.disposed, true);
  assert.equal(released, true);
  assert.equal(registry.size, 0);
  await registry.shutdown();
});
