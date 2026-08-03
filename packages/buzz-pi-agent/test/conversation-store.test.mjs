import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import {
  mkdtemp,
  mkdir,
  readFile,
  readdir,
  rename,
  rm,
  stat,
  symlink,
  unlink,
  utimes,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import {
  ConversationStore,
  deriveNamespace,
  syncDirectoryEntry,
} from "../dist/index.js";
import {
  captureStateLockGeneration,
  removeObservedStaleLock,
} from "../dist/conversation-store.js";
import { silentLogger, testConfig } from "./helpers.mjs";

async function setup(overrides = {}, leaseIdentity) {
  const stateDir = await mkdtemp(join(tmpdir(), "buzz-pi-store-"));
  const config = testConfig({ stateDir, ...overrides });
  const env = {
    BUZZ_PI_NAMESPACE: "test-agent",
    BUZZ_RELAY_URL: "ws://relay.example",
    BUZZ_PRIVATE_KEY: "secret",
  };
  const store = new ConversationStore(config, silentLogger, env, leaseIdentity);
  await store.initialize();
  return { store, stateDir, config, env };
}

async function createSessionFile(directory, name) {
  const path = join(directory, `${name}.jsonl`);
  await writeFile(path, '{"type":"session"}\n', { mode: 0o600 });
  return path;
}

const TEST_LEASE_IDENTITY = Object.freeze({
  hostId: "a".repeat(64),
  bootId: "b".repeat(64),
  pidProbeSafe: true,
});

function contextEvent(piSessionId, message = "Context status.") {
  return {
    type: "context_status",
    timestamp: "2026-08-02T00:00:00.000Z",
    message,
    piSessionId,
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
}

test("explicit namespaces reject traversal and lossy normalization collisions", async () => {
  for (const namespace of [
    ".",
    "..",
    "../outside",
    "team/name",
    "team?name",
    " team",
    "team ",
  ]) {
    assert.throws(
      () => deriveNamespace({ BUZZ_PI_NAMESPACE: namespace }),
      /BUZZ_PI_NAMESPACE/,
      namespace,
    );
  }
  assert.equal(
    deriveNamespace({ BUZZ_PI_NAMESPACE: "team.alpha_1-prod" }),
    "team.alpha_1-prod",
  );

  const parent = await mkdtemp(join(tmpdir(), "buzz-pi-namespace-"));
  const stateDir = join(parent, "state");
  assert.throws(
    () =>
      new ConversationStore(testConfig({ stateDir }), silentLogger, {
        BUZZ_PI_NAMESPACE: "..",
      }),
    /BUZZ_PI_NAMESPACE/,
  );
  await assert.rejects(() => stat(join(parent, "conversations.json")), {
    code: "ENOENT",
  });
});

test("derived namespaces canonicalize equivalent key and relay spellings", () => {
  const secretHex = `${"0".repeat(63)}1`;
  const secretNsec =
    "nsec1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqsmhltgl";

  const canonical = deriveNamespace({
    BUZZ_RELAY_URL: "ws://relay.example/",
    BUZZ_PRIVATE_KEY: secretHex,
  });
  for (const [relayUrl, privateKey] of [
    ["WS://RELAY.EXAMPLE:80", secretNsec],
    ["ws://relay.example", secretHex.toUpperCase()],
    ["ws://relay.example/path/../#ignored", secretNsec],
    ["ws://relay.example", secretNsec.toUpperCase()],
  ]) {
    assert.equal(
      deriveNamespace({
        BUZZ_RELAY_URL: relayUrl,
        BUZZ_PRIVATE_KEY: privateKey,
      }),
      canonical,
    );
  }

  assert.notEqual(
    deriveNamespace({
      BUZZ_RELAY_URL: "wss://relay.example/",
      BUZZ_PRIVATE_KEY: secretHex,
    }),
    canonical,
  );
  assert.notEqual(
    deriveNamespace({
      BUZZ_RELAY_URL: "ws://relay.example/",
      BUZZ_PRIVATE_KEY: `${secretNsec.slice(0, -1)}q`,
    }),
    canonical,
  );
});

test("namespace symlinks cannot redirect durable state outside the configured root", {
  skip: process.platform === "win32",
}, async () => {
  const parent = await mkdtemp(join(tmpdir(), "buzz-pi-namespace-link-"));
  const stateDir = join(parent, "state");
  const outside = join(parent, "outside");
  await mkdir(stateDir, { mode: 0o700 });
  await mkdir(outside, { mode: 0o700 });
  await symlink(outside, join(stateDir, "team"));
  const store = new ConversationStore(testConfig({ stateDir }), silentLogger, {
    BUZZ_PI_NAMESPACE: "team",
  });
  await assert.rejects(() => store.initialize(), /real directory/);
  await assert.rejects(() => stat(join(outside, "conversations.json")), {
    code: "ENOENT",
  });
});

test("Windows skips unsupported directory fsync after atomic manifest rename", async () => {
  // A nonexistent path proves the Windows branch returns before fs.open. The
  // normal-platform branch is exercised by every manifest persistence test.
  await syncDirectoryEntry("Z:\\definitely-missing\\buzz-state", "win32");
});

test("conversation mapping is atomically persisted with private permissions", async () => {
  const { store, stateDir } = await setup();
  const sessionFile = await createSessionFile(stateDir, "pi-one");
  const resolved = await store.resolve(
    "channel:root",
    undefined,
    "/tmp/project",
    async (prior) => {
      assert.equal(prior, undefined);
      return { sessionFile, piSessionId: "pi-one", cwd: "/tmp/project" };
    },
  );
  await resolved.release();

  const manifestPath = join(stateDir, "test-agent", "conversations.json");
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  assert.equal(manifest.conversations["channel:root"].piSessionId, "pi-one");
  assert.equal((await stat(manifestPath)).mode & 0o777, 0o600);
});

test("legacy manifests and outbox records migrate into one stable replay epoch", async () => {
  const { store, stateDir, config, env } = await setup();
  const sessionFile = await createSessionFile(stateDir, "legacy-epoch");
  const session = await store.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async () => ({
      sessionFile,
      piSessionId: "legacy-epoch",
      cwd: "/tmp/project",
    }),
  );
  const eventId = "9ba32f72-e8ce-4195-96a2-7b472198bb7e";
  await store.enqueueSessionEvent(
    "thread",
    eventId,
    contextEvent("legacy-epoch"),
    session.lifecycleGeneration,
  );
  await session.release();

  const manifestPath = join(stateDir, "test-agent", "conversations.json");
  const legacyManifest = JSON.parse(await readFile(manifestPath, "utf8"));
  delete legacyManifest.conversations.thread.lifecycleGeneration;
  await writeFile(manifestPath, `${JSON.stringify(legacyManifest)}\n`, {
    mode: 0o600,
  });
  const pendingRoot = join(stateDir, "test-agent", "pending-events");
  const [conversationDirectory] = await readdir(pendingRoot);
  const eventPath = join(pendingRoot, conversationDirectory, `${eventId}.json`);
  const legacyEvent = JSON.parse(await readFile(eventPath, "utf8"));
  delete legacyEvent.lifecycleGeneration;
  await writeFile(eventPath, `${JSON.stringify(legacyEvent)}\n`, {
    mode: 0o600,
  });

  const restarted = new ConversationStore(config, silentLogger, env);
  await restarted.initialize();
  const migratedMapping = await restarted.get("thread");
  assert.match(migratedMapping.lifecycleGeneration, /^[0-9a-f]{64}$/u);
  const [migratedEvent] = await restarted.listPendingSessionEvents("thread");
  assert.equal(
    migratedEvent.lifecycleGeneration,
    migratedMapping.lifecycleGeneration,
    "an ambiguous legacy notice is conservatively replayable, never dropped",
  );

  const resumed = await restarted.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async (prior) => {
      assert.equal(prior, sessionFile);
      return {
        sessionFile,
        piSessionId: "legacy-epoch-successor",
        cwd: "/tmp/project",
      };
    },
  );
  assert.equal(
    resumed.lifecycleGeneration,
    migratedMapping.lifecycleGeneration,
  );
  const rewritten = JSON.parse(await readFile(manifestPath, "utf8"));
  assert.equal(
    rewritten.conversations.thread.lifecycleGeneration,
    migratedMapping.lifecycleGeneration,
  );
  await resumed.release();
});

test("lifecycle events survive restart and are removed only by idempotent ACK", async () => {
  const { store, stateDir, config, env } = await setup();
  const eventId = "9ba32f72-e8ce-4195-96a2-7b472198bb7e";
  const event = {
    type: "context_status",
    timestamp: "2026-08-02T00:00:00.000Z",
    message: "Context is at 50%.",
    piSessionId: "pi-durable-event",
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
  const sessionFile = await createSessionFile(stateDir, "durable-event");
  const session = await store.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async () => ({
      sessionFile,
      piSessionId: "pi-durable-event",
      cwd: "/tmp/project",
    }),
  );
  await store.enqueueSessionEvent(
    "thread",
    eventId,
    event,
    session.lifecycleGeneration,
  );
  await store.enqueueSessionEvent(
    "thread",
    eventId,
    event,
    session.lifecycleGeneration,
  );
  await session.release();

  const restarted = new ConversationStore(config, silentLogger, env);
  await restarted.initialize();
  const pending = await restarted.listPendingSessionEvents("thread");
  assert.deepEqual(pending, [
    {
      conversationId: "thread",
      eventId,
      lifecycleGeneration: session.lifecycleGeneration,
      event,
      createdAt: pending[0].createdAt,
    },
  ]);
  const pendingRoot = join(stateDir, "test-agent", "pending-events");
  const [conversationDirectory] = await readdir(pendingRoot);
  const [eventFile] = await readdir(join(pendingRoot, conversationDirectory));
  assert.equal(
    (await stat(join(pendingRoot, conversationDirectory, eventFile))).mode &
      0o777,
    0o600,
  );

  await restarted.acknowledgeSessionEvent("thread", eventId);
  await restarted.acknowledgeSessionEvent("thread", eventId);
  assert.deepEqual(await restarted.listPendingSessionEvents("thread"), []);
});

test("lifecycle outbox capacity fails closed without dropping older unacknowledged events", async () => {
  const { store, stateDir } = await setup({ maxPendingSessionEvents: 1 });
  const lifecycleGenerations = new Map();
  const event = {
    type: "session_reset",
    timestamp: "2026-08-02T00:00:00.000Z",
    message: "Started a fresh session.",
    piSessionId: "pi-fresh",
    previousPiSessionId: "pi-old",
    limitTokens: 150_000,
    effectiveLimitTokens: 150_000,
    compactionThresholdTokens: 133_616,
  };
  for (const conversationId of ["thread", "other"]) {
    const sessionFile = await createSessionFile(
      stateDir,
      `event-capacity-${conversationId}`,
    );
    const session = await store.resolve(
      conversationId,
      undefined,
      "/tmp/project",
      async () => ({
        sessionFile,
        piSessionId: "pi-fresh",
        cwd: "/tmp/project",
      }),
    );
    lifecycleGenerations.set(conversationId, session.lifecycleGeneration);
    await session.release();
  }
  const firstId = "9ba32f72-e8ce-4195-96a2-7b472198bb7e";
  await store.enqueueSessionEvent(
    "thread",
    firstId,
    event,
    lifecycleGenerations.get("thread"),
  );
  await assert.rejects(
    () =>
      store.enqueueSessionEvent(
        "other",
        "b8ba08e4-65f5-4aed-9406-6c67fe8375db",
        event,
        lifecycleGenerations.get("other"),
      ),
    /capacity 1 is full/,
  );
  assert.deepEqual(
    (await store.listPendingSessionEvents("thread")).map(
      (record) => record.eventId,
    ),
    [firstId],
  );
  assert.deepEqual(await store.listPendingSessionEvents("other"), []);
});

test("a committed reset supersedes pending lifecycle events and fences late old-generation writes", async () => {
  const { store, stateDir } = await setup();
  const sessionFile = await createSessionFile(stateDir, "event-before-reset");
  const session = await store.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async () => ({
      sessionFile,
      piSessionId: "pi-before-reset",
      cwd: "/tmp/project",
    }),
  );
  const event = {
    type: "context_status",
    timestamp: "2026-08-02T00:00:00.000Z",
    message: "Old context status.",
    piSessionId: "pi-before-reset",
    usedTokens: 120_000,
    remainingTokens: 30_000,
    percent: 80,
    limitTokens: 150_000,
    effectiveLimitTokens: 150_000,
    compactionThresholdTokens: 133_616,
    autoCompaction: true,
    compacting: false,
    model: "provider/model",
  };
  await store.enqueueSessionEvent(
    "thread",
    "9ba32f72-e8ce-4195-96a2-7b472198bb7e",
    event,
    session.lifecycleGeneration,
  );
  assert.equal((await store.listPendingSessionEvents("thread")).length, 1);

  await store.commitReset("thread", "signed-reset");
  assert.deepEqual(await store.listPendingSessionEvents("thread"), []);
  await store.enqueueSessionEvent(
    "thread",
    "b8ba08e4-65f5-4aed-9406-6c67fe8375db",
    event,
    session.lifecycleGeneration,
  );
  assert.deepEqual(
    await store.listPendingSessionEvents("thread"),
    [],
    "a late event from the old Pi generation must not survive reset cleanup",
  );
  await session.release();
});

test("two adapter stores cannot reinstall or replay an old-generation event after reset", async () => {
  const {
    store: oldOwner,
    stateDir,
    config,
    env,
  } = await setup({}, TEST_LEASE_IDENTITY);
  const oldSessionFile = await createSessionFile(stateDir, "event-old-owner");
  const oldSession = await oldOwner.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async () => ({
      sessionFile: oldSessionFile,
      piSessionId: "pi-old-owner",
      cwd: "/tmp/project",
    }),
  );
  const event = {
    type: "context_status",
    timestamp: "2026-08-02T00:00:00.000Z",
    message: "Old owner context status.",
    piSessionId: "pi-old-owner",
    usedTokens: 120_000,
    remainingTokens: 30_000,
    percent: 80,
    limitTokens: 150_000,
    effectiveLimitTokens: 150_000,
    compactionThresholdTokens: 133_616,
    autoCompaction: true,
    compacting: false,
    model: "provider/model",
  };
  assert.equal(
    await oldOwner.enqueueSessionEvent(
      "thread",
      "9ba32f72-e8ce-4195-96a2-7b472198bb7e",
      event,
      oldSession.lifecycleGeneration,
    ),
    true,
  );

  const resetOwner = new ConversationStore(
    config,
    silentLogger,
    env,
    TEST_LEASE_IDENTITY,
  );
  await resetOwner.initialize();
  await resetOwner.commitReset("thread", "signed-reset");
  assert.deepEqual(await resetOwner.listPendingSessionEvents("thread"), []);

  assert.equal(
    await oldOwner.enqueueSessionEvent(
      "thread",
      "b8ba08e4-65f5-4aed-9406-6c67fe8375db",
      event,
      oldSession.lifecycleGeneration,
    ),
    false,
    "the stale adapter must observe the committed manifest generation",
  );

  const freshSessionFile = await createSessionFile(stateDir, "event-new-owner");
  const freshSession = await resetOwner.resolve(
    "thread",
    "signed-reset",
    "/tmp/project",
    async () => ({
      sessionFile: freshSessionFile,
      piSessionId: "pi-new-owner",
      cwd: "/tmp/project",
    }),
  );
  assert.equal(
    await oldOwner.enqueueSessionEvent(
      "thread",
      "a28cf3b0-fdd0-445c-82d1-bbf4de2ab909",
      event,
      oldSession.lifecycleGeneration,
    ),
    false,
    "an installed replacement generation must continue fencing the old owner",
  );
  assert.deepEqual(await resetOwner.listPendingSessionEvents("thread"), []);

  await freshSession.release();
  await oldSession.release();
});

test("replacement cleanup stays inside the conversation fence and cannot delete a successor event", async () => {
  const { store, stateDir, config, env } = await setup();
  const initialFile = await createSessionFile(stateDir, "cleanup-race-a");
  const initial = await store.resolve(
    "thread",
    "reset-a",
    "/tmp/project",
    async () => ({
      sessionFile: initialFile,
      piSessionId: "cleanup-race-a",
      cwd: "/tmp/project",
    }),
  );
  await initial.release();

  const ownerB = new ConversationStore(config, silentLogger, env);
  const ownerC = new ConversationStore(config, silentLogger, env);
  await ownerB.initialize();
  await ownerC.initialize();
  let signalCleanupStarted;
  const cleanupStarted = new Promise((resolve) => {
    signalCleanupStarted = resolve;
  });
  let releaseCleanup;
  const cleanupGate = new Promise((resolve) => {
    releaseCleanup = resolve;
  });
  const originalCleanup = ownerB.clearPendingSessionEvents.bind(ownerB);
  ownerB.clearPendingSessionEvents = async (...args) => {
    signalCleanupStarted();
    await cleanupGate;
    return originalCleanup(...args);
  };

  const fileB = await createSessionFile(stateDir, "cleanup-race-b");
  const resolveB = ownerB.resolve(
    "thread",
    "reset-b",
    "/tmp/project",
    async () => ({
      sessionFile: fileB,
      piSessionId: "cleanup-race-b",
      cwd: "/tmp/project",
    }),
  );
  await cleanupStarted;

  let signalCreateC;
  const createCStarted = new Promise((resolve) => {
    signalCreateC = resolve;
  });
  const fileC = await createSessionFile(stateDir, "cleanup-race-c");
  const successorEventId = "9ba32f72-e8ce-4195-96a2-7b472198bb7e";
  const resolveCAndEnqueue = (async () => {
    const resolved = await ownerC.resolve(
      "thread",
      "reset-c",
      "/tmp/project",
      async () => {
        signalCreateC();
        return {
          sessionFile: fileC,
          piSessionId: "cleanup-race-c",
          cwd: "/tmp/project",
        };
      },
    );
    await ownerC.enqueueSessionEvent(
      "thread",
      successorEventId,
      {
        type: "context_status",
        timestamp: "2026-08-02T00:00:00.000Z",
        message: "Successor context status.",
        piSessionId: "cleanup-race-c",
        usedTokens: 75_000,
        remainingTokens: 75_000,
        percent: 50,
        limitTokens: 150_000,
        effectiveLimitTokens: 150_000,
        compactionThresholdTokens: 133_616,
        autoCompaction: true,
        compacting: false,
        model: "provider/model",
      },
      resolved.lifecycleGeneration,
    );
    return resolved;
  })();
  const createEnteredWhileCleanupBlocked = await Promise.race([
    createCStarted.then(() => true),
    new Promise((resolve) => setTimeout(() => resolve(false), 40)),
  ]);
  releaseCleanup();
  const [resolvedB, resolvedC] = await Promise.all([
    resolveB,
    resolveCAndEnqueue,
  ]);

  assert.equal(
    createEnteredWhileCleanupBlocked,
    false,
    "successor creation entered while predecessor cleanup was still fenced",
  );
  assert.deepEqual(
    (await ownerC.listPendingSessionEvents("thread")).map(
      (pending) => pending.eventId,
    ),
    [successorEventId],
  );
  await resolvedB.release();
  await resolvedC.release();
});

test("cross-host-safe lock owners carry host and boot identity", async () => {
  const { store, stateDir } = await setup();
  const sessionFile = await createSessionFile(stateDir, "lock-owner");
  let owner;
  const resolved = await store.resolve(
    "lock-owner-thread",
    undefined,
    "/tmp/project",
    async () => {
      const locksDirectory = join(stateDir, "test-agent", "locks");
      const lockName = (await readdir(locksDirectory)).find((name) =>
        name.startsWith("conversation-"),
      );
      assert.ok(lockName);
      owner = JSON.parse(
        await readFile(join(locksDirectory, lockName, "owner"), "utf8"),
      );
      return {
        sessionFile,
        piSessionId: "lock-owner",
        cwd: "/tmp/project",
      };
    },
  );
  assert.equal(owner.version, 1);
  assert.equal(owner.pid, process.pid);
  assert.match(owner.token, /^[0-9a-f-]+$/u);
  assert.match(owner.hostId, /^[0-9a-f]{64}$/u);
  if (owner.bootId !== undefined) assert.match(owner.bootId, /^[0-9a-f]{64}$/u);
  assert.ok(Number.isFinite(Date.parse(owner.createdAt)));
  await resolved.release();
});

test("missing persisted JSONL is recreated and mapping is repaired", async () => {
  const { store, stateDir, config, env } = await setup();
  const oldFile = await createSessionFile(stateDir, "old");
  const first = await store.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async () => ({
      sessionFile: oldFile,
      piSessionId: "old",
      cwd: "/tmp/project",
    }),
  );
  await first.release();
  await unlink(oldFile);

  const restarted = new ConversationStore(config, silentLogger, env);
  const newFile = await createSessionFile(stateDir, "new");
  let calls = 0;
  const repaired = await restarted.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async (prior) => {
      calls++;
      if (prior)
        throw Object.assign(new Error("session file missing"), {
          code: "ENOENT",
        });
      return { sessionFile: newFile, piSessionId: "new", cwd: "/tmp/project" };
    },
  );
  assert.equal(calls, 2);
  assert.equal(repaired.mapping?.piSessionId, "new");
  await repaired.release();
});

test("a transcript quota refusal preserves the persisted route and never falls back fresh", async () => {
  const { store, stateDir, config, env } = await setup();
  const oldFile = await createSessionFile(stateDir, "quota-route-old");
  const old = await store.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async () => ({
      sessionFile: oldFile,
      piSessionId: "quota-route-old",
      cwd: "/tmp/project",
    }),
  );
  await old.release();
  const restarted = new ConversationStore(config, silentLogger, env);
  await restarted.initialize();
  let createCalls = 0;
  await assert.rejects(
    () =>
      restarted.resolve("thread", undefined, "/tmp/project", async (prior) => {
        createCalls += 1;
        assert.equal(prior, oldFile);
        throw new Error(
          "BUZZ_SESSION_STORAGE_LIMIT: transcript is full; use /new",
        );
      }),
    /BUZZ_SESSION_STORAGE_LIMIT/,
  );
  assert.equal(createCalls, 1);
  const mapping = await restarted.get("thread");
  assert.equal(mapping?.piSessionId, "quota-route-old");
  assert.equal(mapping?.sessionFile, oldFile);
  await stat(oldFile);
});

test("reset token is idempotent and a changed token atomically repoints context", async () => {
  const { store, stateDir } = await setup();
  const originalFile = await createSessionFile(stateDir, "original");
  const original = await store.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async () => ({
      sessionFile: originalFile,
      piSessionId: "original",
      cwd: "/tmp/project",
    }),
  );
  await original.release();

  const resetFile = await createSessionFile(stateDir, "reset");
  const reset = await store.resolve(
    "thread",
    "reset-event-1",
    "/tmp/project",
    async (prior) => {
      assert.equal(
        prior,
        undefined,
        "new reset token must never reopen old context",
      );
      return {
        sessionFile: resetFile,
        piSessionId: "reset",
        cwd: "/tmp/project",
      };
    },
  );
  assert.equal(reset.previousPiSessionId, "original");
  assert.equal(reset.skipRelayHistory, true);
  await reset.release();

  const sameToken = await store.resolve(
    "thread",
    "reset-event-1",
    "/tmp/project",
    async (prior) => {
      assert.equal(
        prior,
        resetFile,
        "same reset token must converge on the reset context",
      );
      return {
        sessionFile: resetFile,
        piSessionId: "reset",
        cwd: "/tmp/project",
      };
    },
  );
  assert.equal(sameToken.previousPiSessionId, undefined);
  assert.equal(sameToken.skipRelayHistory, false);
  await sameToken.release();
});

test("a reset tombstone survives restart and skips old relay history once fresh context is installed", async () => {
  const { store, stateDir, config, env } = await setup();
  const oldFile = await createSessionFile(stateDir, "before-forget");
  const active = await store.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async () => ({
      sessionFile: oldFile,
      piSessionId: "before-forget",
      cwd: "/tmp/project",
    }),
  );

  assert.equal(await active.forget(), oldFile);
  await active.release();
  const manifestPath = join(stateDir, "test-agent", "conversations.json");
  const afterForget = JSON.parse(await readFile(manifestPath, "utf8"));
  assert.equal(afterForget.conversations.thread, undefined);
  assert.equal(
    afterForget.resetTombstones.thread.previousPiSessionId,
    "before-forget",
  );

  // Simulate both adapter and outer harness restarting after reset ACK but
  // before a replacement session/new carried the original reset token.
  const restarted = new ConversationStore(config, silentLogger, env);
  await restarted.initialize();
  const freshFile = await createSessionFile(stateDir, "after-restart");
  const fresh = await restarted.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async (prior) => {
      assert.equal(prior, undefined);
      return {
        sessionFile: freshFile,
        piSessionId: "after-restart",
        cwd: "/tmp/project",
      };
    },
  );
  assert.equal(fresh.resumed, false);
  assert.equal(fresh.skipRelayHistory, true);
  assert.equal(fresh.previousPiSessionId, "before-forget");
  await fresh.release();

  const installed = JSON.parse(await readFile(manifestPath, "utf8"));
  assert.equal(installed.resetTombstones.thread.status, "retained");
  assert.equal(
    installed.resetTombstones.thread.installedPiSessionId,
    "after-restart",
  );
  assert.equal(installed.conversations.thread.piSessionId, "after-restart");
  assert.equal(installed.conversations.thread.relayHistoryCleared, true);

  const resumedStore = new ConversationStore(config, silentLogger, env);
  await resumedStore.initialize();
  const resumed = await resumedStore.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async (prior) => {
      assert.equal(prior, freshFile);
      return {
        sessionFile: freshFile,
        piSessionId: "after-restart",
        cwd: "/tmp/project",
      };
    },
  );
  assert.equal(resumed.resumed, true);
  assert.equal(resumed.skipRelayHistory, false);
  await resumed.release();
});

test("conversation-level reset commit is idempotent for cold mappings and installed fresh generations", async () => {
  const { store, stateDir } = await setup();
  const oldFile = await createSessionFile(stateDir, "cold-reset-old");
  const old = await store.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async () => ({
      sessionFile: oldFile,
      piSessionId: "cold-reset-old",
      cwd: "/tmp/project",
    }),
  );
  await old.release();

  assert.deepEqual(await store.commitReset("thread", "signed-reset-1"), {
    alreadyCommitted: false,
    disposeLiveSession: true,
    retiredSessionFile: oldFile,
  });
  assert.deepEqual(await store.commitReset("thread", "signed-reset-1"), {
    alreadyCommitted: true,
    disposeLiveSession: true,
  });

  const freshFile = await createSessionFile(stateDir, "cold-reset-fresh");
  const fresh = await store.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async (prior) => {
      assert.equal(prior, undefined);
      return {
        sessionFile: freshFile,
        piSessionId: "cold-reset-fresh",
        cwd: "/tmp/project",
      };
    },
  );
  assert.equal(fresh.skipRelayHistory, true);
  assert.notEqual(fresh.lifecycleGeneration, old.lifecycleGeneration);
  const freshEventId = "9ba32f72-e8ce-4195-96a2-7b472198bb7e";
  const freshEvent = {
    type: "context_status",
    timestamp: "2026-08-02T00:00:00.000Z",
    message: "Fresh reset generation context status.",
    piSessionId: "cold-reset-fresh",
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
  await store.enqueueSessionEvent(
    "thread",
    freshEventId,
    freshEvent,
    fresh.lifecycleGeneration,
  );
  await fresh.release();

  assert.deepEqual(await store.commitReset("thread", "signed-reset-1"), {
    alreadyCommitted: true,
    disposeLiveSession: false,
  });
  assert.equal((await store.get("thread"))?.piSessionId, "cold-reset-fresh");
  assert.deepEqual(
    (await store.listPendingSessionEvents("thread")).map(
      (pending) => pending.eventId,
    ),
    [freshEventId],
    "an idempotent reset retry must not clear the installed epoch's outbox",
  );
});

test("pending reset capacity is recoverable and never drops a required barrier", async () => {
  const { store, stateDir } = await setup({
    maxPendingResetTombstones: 2,
    maxRetainedResetTombstones: 1,
    // This test exercises reset-barrier capacity. Keep enough mapping slots
    // that the independent persisted-conversation hard cap is not the reason
    // a pending reset cannot be consumed.
    maxPersistedConversations: 2,
  });
  await store.commitReset("a", "reset-a");
  await store.commitReset("b", "reset-b");
  await assert.rejects(
    () => store.commitReset("c", "reset-c"),
    /Pending reset tombstone capacity 2 is full/,
  );

  const aFile = await createSessionFile(stateDir, "bounded-a");
  const a = await store.resolve(
    "a",
    undefined,
    "/tmp/project",
    async (prior) => {
      assert.equal(prior, undefined);
      return {
        sessionFile: aFile,
        piSessionId: "bounded-a",
        cwd: "/tmp/project",
      };
    },
  );
  assert.equal(a.skipRelayHistory, true);
  await a.release();
  await store.commitReset("c", "reset-c");

  const bFile = await createSessionFile(stateDir, "bounded-b");
  const b = await store.resolve(
    "b",
    undefined,
    "/tmp/project",
    async (prior) => {
      assert.equal(prior, undefined);
      return {
        sessionFile: bFile,
        piSessionId: "bounded-b",
        cwd: "/tmp/project",
      };
    },
  );
  await b.release();

  // Filling the second mapping slot proves the reset barrier remains
  // recoverable when the next insert forces the oldest cleared mapping out.
  const cFile = await createSessionFile(stateDir, "bounded-c");
  const c = await store.resolve(
    "c",
    undefined,
    "/tmp/project",
    async (prior) => {
      assert.equal(prior, undefined);
      return {
        sessionFile: cFile,
        piSessionId: "bounded-c",
        cwd: "/tmp/project",
      };
    },
  );
  await c.release();

  const manifestPath = join(stateDir, "test-agent", "conversations.json");
  const afterPrune = JSON.parse(await readFile(manifestPath, "utf8"));
  assert.equal(afterPrune.conversations.a, undefined);
  assert.equal(afterPrune.resetTombstones.a.status, "pending");
  assert.equal(afterPrune.conversations.b.piSessionId, "bounded-b");
  assert.equal(afterPrune.conversations.c.piSessionId, "bounded-c");
  assert.equal(afterPrune.resetTombstones.c.status, "retained");

  const aFreshFile = await createSessionFile(stateDir, "bounded-a-fresh");
  const aFresh = await store.resolve(
    "a",
    undefined,
    "/tmp/project",
    async (prior) => {
      assert.equal(prior, undefined);
      return {
        sessionFile: aFreshFile,
        piSessionId: "bounded-a-fresh",
        cwd: "/tmp/project",
      };
    },
  );
  assert.equal(aFresh.skipRelayHistory, true);
  await aFresh.release();
});

test("retained tombstone TTL pruning leaves mapping safety that reactivates on cold prune", async () => {
  const { store, stateDir, config, env } = await setup();
  const oldFile = await createSessionFile(stateDir, "ttl-reset-old");
  const active = await store.resolve(
    "thread",
    "reset-ttl",
    "/tmp/project",
    async () => ({
      sessionFile: oldFile,
      piSessionId: "ttl-reset-old",
      cwd: "/tmp/project",
    }),
  );
  await active.release();

  const manifestPath = join(stateDir, "test-agent", "conversations.json");
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  manifest.conversations.thread.lastUsedAt = "2000-01-01T00:00:00.000Z";
  manifest.resetTombstones.thread.consumedAt = "2000-01-01T00:00:00.000Z";
  await writeFile(manifestPath, `${JSON.stringify(manifest)}\n`, {
    mode: 0o600,
  });

  const restarted = new ConversationStore(
    {
      ...config,
      persistedConversationTtlMs: 1,
      resetTombstoneTtlMs: 1,
    },
    silentLogger,
    env,
  );
  await restarted.initialize();
  const pruned = JSON.parse(await readFile(manifestPath, "utf8"));
  assert.equal(pruned.conversations.thread, undefined);
  assert.equal(pruned.resetTombstones.thread.status, "pending");
  await assert.rejects(() => stat(oldFile), { code: "ENOENT" });

  const freshFile = await createSessionFile(stateDir, "ttl-reset-fresh");
  const fresh = await restarted.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async (prior) => {
      assert.equal(prior, undefined);
      return {
        sessionFile: freshFile,
        piSessionId: "ttl-reset-fresh",
        cwd: "/tmp/project",
      };
    },
  );
  assert.equal(fresh.skipRelayHistory, true);
  await fresh.release();
});

test("late forget from an old ACP session cannot delete a newer reset mapping", async () => {
  const { store, stateDir } = await setup();
  const oldFile = await createSessionFile(stateDir, "old");
  const old = await store.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async () => ({
      sessionFile: oldFile,
      piSessionId: "old",
      cwd: "/tmp/project",
    }),
  );
  await old.release();
  const newFile = await createSessionFile(stateDir, "new");
  const current = await store.resolve(
    "thread",
    "reset-2",
    "/tmp/project",
    async () => ({
      sessionFile: newFile,
      piSessionId: "new",
      cwd: "/tmp/project",
    }),
  );
  const eventId = "9ba32f72-e8ce-4195-96a2-7b472198bb7e";
  await store.enqueueSessionEvent(
    "thread",
    eventId,
    {
      type: "context_status",
      timestamp: "2026-08-02T00:00:00.000Z",
      message: "Replacement context status.",
      piSessionId: "new",
      usedTokens: 75_000,
      remainingTokens: 75_000,
      percent: 50,
      limitTokens: 150_000,
      effectiveLimitTokens: 150_000,
      compactionThresholdTokens: 133_616,
      autoCompaction: true,
      compacting: false,
      model: "provider/model",
    },
    current.lifecycleGeneration,
  );

  assert.equal(await old.forget(), undefined);
  assert.equal((await store.get("thread"))?.piSessionId, "new");
  assert.deepEqual(
    (await store.listPendingSessionEvents("thread")).map(
      (pending) => pending.eventId,
    ),
    [eventId],
  );
  await current.release();
});

test("late forget from an old lease owner preserves the resumed mapping and its lifecycle outbox", async () => {
  const { store, stateDir } = await setup();
  const sessionFile = await createSessionFile(stateDir, "same-pi-generation");
  const replacementFile = await createSessionFile(
    stateDir,
    "same-pi-replacement",
  );
  const first = await store.resolve(
    "thread",
    "retained-reset-token",
    "/tmp/project",
    async () => ({
      sessionFile,
      piSessionId: "same-pi-generation",
      cwd: "/tmp/project",
    }),
  );
  await first.release();
  const resumed = await store.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async (prior) => {
      assert.equal(prior, sessionFile);
      return {
        sessionFile: replacementFile,
        piSessionId: "same-pi-generation",
        cwd: "/tmp/project",
      };
    },
  );
  const eventId = "9ba32f72-e8ce-4195-96a2-7b472198bb7e";
  await store.enqueueSessionEvent(
    "thread",
    eventId,
    {
      type: "context_status",
      timestamp: "2026-08-02T00:00:00.000Z",
      message: "Replacement owner context status.",
      piSessionId: "same-pi-generation",
      usedTokens: 75_000,
      remainingTokens: 75_000,
      percent: 50,
      limitTokens: 150_000,
      effectiveLimitTokens: 150_000,
      compactionThresholdTokens: 133_616,
      autoCompaction: true,
      compacting: false,
      model: "provider/model",
    },
    resumed.lifecycleGeneration,
  );

  assert.equal(await first.forget(), undefined);
  assert.equal(resumed.mapping.lastResetToken, "retained-reset-token");
  assert.equal(
    (await store.get("thread"))?.lease?.ownerId,
    resumed.mapping.lease.ownerId,
  );
  await stat(replacementFile);
  assert.deepEqual(
    (await store.listPendingSessionEvents("thread")).map(
      (pending) => pending.eventId,
    ),
    [eventId],
  );

  assert.equal(await resumed.forget(), replacementFile);
  assert.deepEqual(await store.listPendingSessionEvents("thread"), []);
  await resumed.release();
  await assert.rejects(() => stat(replacementFile), { code: "ENOENT" });
  await unlink(sessionFile);
});

test("workspace cwd change never resumes a context from another checkout", async () => {
  const { store, stateDir } = await setup();
  const firstFile = await createSessionFile(stateDir, "cwd-a");
  const first = await store.resolve(
    "thread",
    undefined,
    "/tmp/a",
    async () => ({
      sessionFile: firstFile,
      piSessionId: "a",
      cwd: "/tmp/a",
    }),
  );
  await first.release();
  const secondFile = await createSessionFile(stateDir, "cwd-b");
  const second = await store.resolve(
    "thread",
    undefined,
    "/tmp/b",
    async (prior) => {
      assert.equal(prior, undefined);
      return { sessionFile: secondFile, piSessionId: "b", cwd: "/tmp/b" };
    },
  );
  assert.equal(second.previousPiSessionId, "a");
  assert.equal(
    second.lifecycleGeneration,
    first.lifecycleGeneration,
    "a workspace recovery changes Pi context but not the durable lifecycle epoch",
  );
  const recoveryEventId = "9ba32f72-e8ce-4195-96a2-7b472198bb7e";
  assert.equal(
    await store.enqueueSessionEvent(
      "thread",
      recoveryEventId,
      {
        type: "context_status",
        timestamp: "2026-08-02T00:00:00.000Z",
        message: "Late status from the prior Pi process.",
        piSessionId: "a",
        usedTokens: 75_000,
        remainingTokens: 75_000,
        percent: 50,
        limitTokens: 150_000,
        effectiveLimitTokens: 150_000,
        compactionThresholdTokens: 133_616,
        autoCompaction: true,
        compacting: false,
        model: "provider/model",
      },
      first.lifecycleGeneration,
    ),
    true,
  );
  assert.equal(
    (await store.listPendingSessionEvents("thread"))[0].lifecycleGeneration,
    second.lifecycleGeneration,
  );
  await second.release();
});

test("pruning removes oldest inactive mappings and their JSONL files", async () => {
  const { store, stateDir } = await setup({
    maxPersistedConversations: 1,
    persistedConversationTtlMs: 60_000,
  });
  const firstFile = await createSessionFile(stateDir, "first");
  const first = await store.resolve(
    "first",
    undefined,
    "/tmp/project",
    async () => ({
      sessionFile: firstFile,
      piSessionId: "first",
      cwd: "/tmp/project",
    }),
  );
  await first.release();
  await new Promise((resolve) => setTimeout(resolve, 5));
  const secondFile = await createSessionFile(stateDir, "second");
  const second = await store.resolve(
    "second",
    undefined,
    "/tmp/project",
    async () => ({
      sessionFile: secondFile,
      piSessionId: "second",
      cwd: "/tmp/project",
    }),
  );
  await second.release();
  await store.prune(new Set());

  assert.equal(await store.get("first"), undefined);
  assert.equal((await store.get("second"))?.piSessionId, "second");
  await assert.rejects(() => stat(firstFile), { code: "ENOENT" });
});

test("unacknowledged lifecycle events pin TTL and capacity pruning until ACK", async () => {
  const { store, stateDir, config, env } = await setup({
    maxPersistedConversations: 1,
    persistedConversationTtlMs: 1,
  });
  const sessionFile = await createSessionFile(stateDir, "pinned-outbox");
  const session = await store.resolve(
    "pinned",
    undefined,
    "/tmp/project",
    async () => ({
      sessionFile,
      piSessionId: "pinned-outbox",
      cwd: "/tmp/project",
    }),
  );
  const eventId = "9ba32f72-e8ce-4195-96a2-7b472198bb7e";
  await store.enqueueSessionEvent(
    "pinned",
    eventId,
    contextEvent("pinned-outbox"),
    session.lifecycleGeneration,
  );
  await session.release();
  const manifestPath = join(stateDir, "test-agent", "conversations.json");
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  manifest.conversations.pinned.lastUsedAt = "2000-01-01T00:00:00.000Z";
  await writeFile(manifestPath, `${JSON.stringify(manifest)}\n`, {
    mode: 0o600,
  });

  assert.equal(await store.prune(new Set()), 0);
  await stat(sessionFile);
  assert.equal((await store.get("pinned"))?.piSessionId, "pinned-outbox");
  let createCalls = 0;
  await assert.rejects(
    () =>
      store.resolve("blocked", undefined, "/tmp/project", async () => {
        createCalls++;
        throw new Error("must not create");
      }),
    /capacity 1 is full/,
  );
  assert.equal(createCalls, 0);

  const restarted = new ConversationStore(config, silentLogger, env);
  await restarted.initialize();
  assert.equal((await restarted.get("pinned"))?.piSessionId, "pinned-outbox");
  assert.deepEqual(
    (await restarted.listPendingSessionEvents("pinned")).map(
      (pending) => pending.eventId,
    ),
    [eventId],
  );
  await restarted.acknowledgeSessionEvent("pinned", eventId);
  assert.equal(await restarted.prune(new Set()), 1);
  assert.equal(await restarted.get("pinned"), undefined);
  await assert.rejects(() => stat(sessionFile), { code: "ENOENT" });
});

test("capacity pruning skips an older pinned mapping and selects the next safe victim", async () => {
  const { store, stateDir } = await setup({
    maxPersistedConversations: 2,
    persistedConversationTtlMs: 60_000,
  });
  const fileA = await createSessionFile(stateDir, "capacity-pinned-a");
  const a = await store.resolve("a", undefined, "/tmp/project", async () => ({
    sessionFile: fileA,
    piSessionId: "capacity-pinned-a",
    cwd: "/tmp/project",
  }));
  await store.enqueueSessionEvent(
    "a",
    "9ba32f72-e8ce-4195-96a2-7b472198bb7e",
    contextEvent("capacity-pinned-a"),
    a.lifecycleGeneration,
  );
  await a.release();
  await new Promise((resolve) => setTimeout(resolve, 5));
  const fileB = await createSessionFile(stateDir, "capacity-victim-b");
  const b = await store.resolve("b", undefined, "/tmp/project", async () => ({
    sessionFile: fileB,
    piSessionId: "capacity-victim-b",
    cwd: "/tmp/project",
  }));
  await b.release();

  const fileC = await createSessionFile(stateDir, "capacity-successor-c");
  const c = await store.resolve("c", undefined, "/tmp/project", async () => ({
    sessionFile: fileC,
    piSessionId: "capacity-successor-c",
    cwd: "/tmp/project",
  }));
  await c.release();

  assert.equal((await store.get("a"))?.piSessionId, "capacity-pinned-a");
  assert.equal(await store.get("b"), undefined);
  assert.equal((await store.get("c"))?.piSessionId, "capacity-successor-c");
  await stat(fileA);
  await assert.rejects(() => stat(fileB), { code: "ENOENT" });
  assert.equal((await store.listPendingSessionEvents("a")).length, 1);
});

test("prune and enqueue serialize so no accepted event can lose its mapping", async () => {
  const { store, stateDir } = await setup({
    persistedConversationTtlMs: 1,
  });
  const sessionFile = await createSessionFile(stateDir, "prune-enqueue-race");
  const session = await store.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async () => ({
      sessionFile,
      piSessionId: "prune-enqueue-race",
      cwd: "/tmp/project",
    }),
  );
  await session.release();
  const manifestPath = join(stateDir, "test-agent", "conversations.json");
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  manifest.conversations.thread.lastUsedAt = "2000-01-01T00:00:00.000Z";
  await writeFile(manifestPath, `${JSON.stringify(manifest)}\n`, {
    mode: 0o600,
  });

  let signalPruneCheck;
  const pruneCheckStarted = new Promise((resolve) => {
    signalPruneCheck = resolve;
  });
  let releasePruneCheck;
  const pruneCheckGate = new Promise((resolve) => {
    releasePruneCheck = resolve;
  });
  const originalHasPending = store.hasPendingSessionEventsLocked.bind(store);
  store.hasPendingSessionEventsLocked = async (...args) => {
    signalPruneCheck();
    await pruneCheckGate;
    return originalHasPending(...args);
  };

  const pruning = store.prune(new Set());
  await pruneCheckStarted;
  let enqueueSettled = false;
  const enqueue = store
    .enqueueSessionEvent(
      "thread",
      "9ba32f72-e8ce-4195-96a2-7b472198bb7e",
      contextEvent("prune-enqueue-race"),
      session.lifecycleGeneration,
    )
    .finally(() => {
      enqueueSettled = true;
    });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(enqueueSettled, false, "enqueue bypassed the prune outbox lock");
  releasePruneCheck();

  assert.equal(await pruning, 1);
  assert.equal(await enqueue, false);
  assert.equal(await store.get("thread"), undefined);
  assert.deepEqual(await store.listPendingSessionEvents("thread"), []);
  await assert.rejects(() => stat(sessionFile), { code: "ENOENT" });
});

test("corrupt manifest fails closed across restart and never creates fresh context", async () => {
  const { stateDir, config, env } = await setup();
  const directory = join(stateDir, "test-agent");
  await mkdir(directory, { recursive: true });
  await writeFile(join(directory, "conversations.json"), "not-json", {
    mode: 0o600,
  });
  const restarted = new ConversationStore(config, silentLogger, env);
  await assert.rejects(
    () => restarted.initialize(),
    /refusing to lose durable reset boundaries/,
  );
  let createCalls = 0;
  await assert.rejects(
    () =>
      restarted.resolve("anything", undefined, "/tmp/project", async () => {
        createCalls++;
        throw new Error("must never create from corrupt state");
      }),
    /refusing to lose durable reset boundaries/,
  );
  assert.equal(createCalls, 0);
  assert.equal(
    await readFile(join(directory, "conversations.json"), "utf8"),
    "not-json",
  );
});

test("a manifest missing after durable initialization fails closed", async () => {
  const { stateDir, config, env } = await setup();
  const directory = join(stateDir, "test-agent");
  const manifestPath = join(directory, "conversations.json");
  await stat(join(directory, ".buzz-pi-state-v1"));
  await unlink(manifestPath);
  await writeFile(join(directory, ".conversations.json.crash.tmp"), "partial", {
    mode: 0o600,
  });

  const restarted = new ConversationStore(config, silentLogger, env);
  await assert.rejects(
    () => restarted.initialize(),
    /manifest is missing after durable initialization/,
  );
  await assert.rejects(() => stat(manifestPath), { code: "ENOENT" });
});

test("an intact manifest safely repairs a missing initialization marker", async () => {
  const { stateDir, config, env } = await setup();
  const markerPath = join(stateDir, "test-agent", ".buzz-pi-state-v1");
  await unlink(markerPath);
  const restarted = new ConversationStore(config, silentLogger, env);
  await restarted.initialize();
  assert.equal(await readFile(markerPath, "utf8"), "buzz-pi-state-v1\n");
});

test("a newer committed reset token rejects a stale cross-adapter session open", async () => {
  const { store, stateDir, config, env } = await setup();
  const helper = new ConversationStore(config, silentLogger, env);
  await helper.initialize();
  await helper.commitReset("thread", "reset-newer-b");
  let createCalls = 0;
  await assert.rejects(
    () =>
      store.resolve("thread", "reset-stale-a", "/tmp/project", async () => {
        createCalls += 1;
        return {
          sessionFile: await createSessionFile(stateDir, "must-not-open"),
          piSessionId: "must-not-open",
          cwd: "/tmp/project",
        };
      }),
    /must match the latest committed Buzz reset token/,
  );
  assert.equal(createCalls, 0);
  const manifest = JSON.parse(
    await readFile(join(stateDir, "test-agent", "conversations.json"), "utf8"),
  );
  assert.equal(manifest.conversations.thread, undefined);
  assert.equal(manifest.resetTombstones.thread.status, "pending");
  assert.equal(manifest.resetTombstones.thread.resetToken, "reset-newer-b");
});

test("oversized manifest serialization is rejected before rename and preserves the previous manifest", async () => {
  const { stateDir, config, env } = await setup();
  const resetTombstones = {};
  for (let index = 0; index < 20_800; index++) {
    const conversationId = `bulk-${String(index).padStart(5, "0")}-${"c".repeat(80)}`;
    resetTombstones[conversationId] = {
      conversationId,
      resetToken: "r".repeat(512),
      createdAt: "2026-08-02T00:00:00.000Z",
    };
  }
  const manifest = { version: 1, conversations: {}, resetTombstones };
  const compact = `${JSON.stringify(manifest)}\n`;
  const prettyBytes = Buffer.byteLength(
    `${JSON.stringify(manifest, null, 2)}\n`,
  );
  assert.ok(Buffer.byteLength(compact) < 16 * 1024 * 1024);
  assert.ok(prettyBytes > 16 * 1024 * 1024);

  const manifestPath = join(stateDir, "test-agent", "conversations.json");
  await writeFile(manifestPath, compact, { mode: 0o600 });
  const restarted = new ConversationStore(config, silentLogger, env);
  await assert.rejects(
    () => restarted.touch("new-thread", "missing", undefined),
    /manifest serialization exceeds 16777216 bytes/,
  );
  assert.equal(
    await readFile(manifestPath, "utf8"),
    compact,
    "failed serialization must leave the prior atomic manifest untouched",
  );
});

test("a changed signed reset token supersedes an active cross-adapter lease safely", async () => {
  const { store: firstStore, stateDir, config, env } = await setup();
  const oldFile = await createSessionFile(stateDir, "active-old");
  const old = await firstStore.resolve(
    "thread",
    "reset-1",
    "/tmp/project",
    async () => ({
      sessionFile: oldFile,
      piSessionId: "old",
      cwd: "/tmp/project",
    }),
  );

  const secondStore = new ConversationStore(config, silentLogger, env);
  await secondStore.initialize();
  await assert.rejects(
    () =>
      secondStore.resolve("thread", "reset-1", "/tmp/project", async () => {
        throw new Error("must not create");
      }),
    /active in another Pi runtime/,
  );

  const newFile = await createSessionFile(stateDir, "active-new");
  const reset = await secondStore.resolve(
    "thread",
    "reset-2",
    "/tmp/project",
    async (prior) => {
      assert.equal(prior, undefined);
      return { sessionFile: newFile, piSessionId: "new", cwd: "/tmp/project" };
    },
  );
  assert.equal(reset.previousPiSessionId, "old");
  assert.equal(
    reset.retiredSessionFile,
    undefined,
    "an active writer's JSONL is retained",
  );

  await old.release();
  await firstStore.touch("thread", "old", "reset-1");
  assert.equal(await old.forget(), undefined);
  const current = await secondStore.get("thread");
  assert.equal(current?.piSessionId, "new");
  assert.equal(current?.lastResetToken, "reset-2");
  assert.ok(
    current?.lease,
    "late old release must not clear the replacement lease",
  );
  await assert.rejects(() => stat(oldFile), { code: "ENOENT" });
  await stat(newFile);
  await reset.release();
});

test("a future lease from a confirmed-dead local PID is recovered immediately", async () => {
  const leaseIdentity = {
    hostId: "a".repeat(64),
    bootId: "b".repeat(64),
    pidProbeSafe: true,
  };
  const { store, stateDir, config, env } = await setup({}, leaseIdentity);
  const sessionFile = await createSessionFile(stateDir, "crash-recovery");
  await store.resolve("thread", undefined, "/tmp/project", async () => ({
    sessionFile,
    piSessionId: "before-crash",
    cwd: "/tmp/project",
  }));

  const child = spawn(process.execPath, ["-e", ""], { stdio: "ignore" });
  assert.ok(child.pid, "spawned process must expose its PID");
  const deadPid = child.pid;
  await once(child, "exit");
  assert.throws(
    () => process.kill(deadPid, 0),
    (error) => error?.code === "ESRCH",
    "fixture PID must be definitively dead on this host",
  );

  const manifestPath = join(stateDir, "test-agent", "conversations.json");
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  manifest.conversations.thread.lease.pid = deadPid;
  manifest.conversations.thread.lease.expiresAt = new Date(
    Date.now() + 60 * 60 * 1_000,
  ).toISOString();
  await writeFile(manifestPath, `${JSON.stringify(manifest)}\n`, {
    mode: 0o600,
  });

  const restarted = new ConversationStore(
    config,
    silentLogger,
    env,
    leaseIdentity,
  );
  await restarted.initialize();
  const recovered = await restarted.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async (prior) => {
      assert.equal(prior, sessionFile);
      return {
        sessionFile,
        piSessionId: "before-crash",
        cwd: "/tmp/project",
      };
    },
  );
  assert.equal(recovered.resumed, true);
  await recovered.release();
});

test("an expired same-boot lease cannot be kept alive by PID reuse", async () => {
  const leaseIdentity = TEST_LEASE_IDENTITY;
  const { store, stateDir, config, env } = await setup({}, leaseIdentity);
  const sessionFile = await createSessionFile(stateDir, "pid-reuse");
  const original = await store.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async () => ({
      sessionFile,
      piSessionId: "pid-reuse",
      cwd: "/tmp/project",
    }),
  );
  const manifestPath = join(stateDir, "test-agent", "conversations.json");
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  manifest.conversations.thread.lease.pid = process.pid;
  manifest.conversations.thread.lease.expiresAt = "2000-01-01T00:00:00.000Z";
  await writeFile(manifestPath, `${JSON.stringify(manifest)}\n`, {
    mode: 0o600,
  });

  const restarted = new ConversationStore(
    config,
    silentLogger,
    env,
    leaseIdentity,
  );
  await restarted.initialize();
  const recovered = await restarted.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async (prior) => {
      assert.equal(
        prior,
        undefined,
        "an expired live-PID lease must isolate takeover onto fresh JSONL",
      );
      return {
        sessionFile: await createSessionFile(stateDir, "pid-reuse-fresh"),
        piSessionId: "pid-reuse-fresh",
        cwd: "/tmp/project",
      };
    },
  );
  assert.equal(recovered.resumed, false);
  assert.equal(
    await original.refresh(),
    false,
    "a stale runtime generation must not renew its successor's lease",
  );
  await original.release();
  assert.equal((await restarted.get("thread"))?.piSessionId, "pid-reuse-fresh");
  await recovered.release();
});

test("an expired foreign-host lease takes over onto a fresh JSONL and fences the old generation", async () => {
  const {
    store: first,
    stateDir,
    config,
    env,
  } = await setup({}, TEST_LEASE_IDENTITY);
  const oldFile = await createSessionFile(stateDir, "foreign-old");
  const old = await first.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async () => ({
      sessionFile: oldFile,
      piSessionId: "foreign-old",
      cwd: "/tmp/project",
    }),
  );
  const manifestPath = join(stateDir, "test-agent", "conversations.json");
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  manifest.conversations.thread.lease.expiresAt = "2000-01-01T00:00:00.000Z";
  await writeFile(manifestPath, `${JSON.stringify(manifest)}\n`, {
    mode: 0o600,
  });

  const foreignIdentity = {
    ...TEST_LEASE_IDENTITY,
    hostId: "f".repeat(64),
  };
  const second = new ConversationStore(
    config,
    silentLogger,
    env,
    foreignIdentity,
  );
  await second.initialize();
  const freshFile = await createSessionFile(stateDir, "foreign-fresh");
  const fresh = await second.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async (prior) => {
      assert.equal(prior, undefined);
      return {
        sessionFile: freshFile,
        piSessionId: "foreign-fresh",
        cwd: "/tmp/project",
      };
    },
  );
  assert.equal(fresh.resumed, false);
  assert.equal(await old.refresh(), false);
  await old.release();
  assert.equal((await second.get("thread"))?.piSessionId, "foreign-fresh");
  await fresh.release();
});

test("a delayed resolver cannot publish after its conversation lock generation is stolen", async () => {
  const {
    store: first,
    stateDir,
    config,
    env,
  } = await setup({}, TEST_LEASE_IDENTITY);
  const second = new ConversationStore(config, silentLogger, env, {
    ...TEST_LEASE_IDENTITY,
    hostId: "f".repeat(64),
  });
  await second.initialize();
  const firstFile = await createSessionFile(stateDir, "lock-race-first");
  const secondFile = await createSessionFile(stateDir, "lock-race-second");
  let unblockFirst;
  const firstBlocked = new Promise((resolve) => {
    unblockFirst = resolve;
  });
  let firstEntered;
  const firstHasLock = new Promise((resolve) => {
    firstEntered = resolve;
  });
  const firstResolve = first.resolve(
    "lock-race",
    undefined,
    "/tmp/project",
    async () => {
      firstEntered();
      await firstBlocked;
      return {
        sessionFile: firstFile,
        piSessionId: "lock-race-first",
        cwd: "/tmp/project",
      };
    },
  );
  await firstHasLock;

  const locksDirectory = join(stateDir, "test-agent", "locks");
  const conversationLock = (await readdir(locksDirectory)).find((name) =>
    name.startsWith("conversation-"),
  );
  assert.ok(conversationLock);
  const lockDirectory = join(locksDirectory, conversationLock);
  const heartbeat = (await readdir(lockDirectory)).find((name) =>
    name.startsWith("heartbeat-"),
  );
  assert.ok(heartbeat);
  const stale = new Date(Date.now() - 5 * 60_000);
  await utimes(join(lockDirectory, heartbeat), stale, stale);

  const successor = await second.resolve(
    "lock-race",
    undefined,
    "/tmp/project",
    async () => ({
      sessionFile: secondFile,
      piSessionId: "lock-race-second",
      cwd: "/tmp/project",
    }),
  );
  unblockFirst();
  await assert.rejects(firstResolve, /lock ownership was lost/);
  assert.equal(
    (await second.get("lock-race"))?.piSessionId,
    "lock-race-second",
  );
  await successor.release();
});

test("persisted conversation capacity fails before create when every victim may still write", async () => {
  const { store, stateDir } = await setup(
    { maxPersistedConversations: 1 },
    TEST_LEASE_IDENTITY,
  );
  const firstFile = await createSessionFile(stateDir, "capacity-first");
  const first = await store.resolve(
    "first",
    undefined,
    "/tmp/project",
    async () => ({
      sessionFile: firstFile,
      piSessionId: "capacity-first",
      cwd: "/tmp/project",
    }),
  );
  let createCalls = 0;
  await assert.rejects(
    () =>
      store.resolve("second", undefined, "/tmp/project", async () => {
        createCalls += 1;
        return {
          sessionFile: await createSessionFile(stateDir, "capacity-second"),
          piSessionId: "capacity-second",
          cwd: "/tmp/project",
        };
      }),
    /capacity 1 is full/,
  );
  assert.equal(createCalls, 0);

  const manifestPath = join(stateDir, "test-agent", "conversations.json");
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  manifest.conversations.first.lease.expiresAt = "2000-01-01T00:00:00.000Z";
  await writeFile(manifestPath, `${JSON.stringify(manifest)}\n`, {
    mode: 0o600,
  });
  await assert.rejects(
    () =>
      store.resolve("second", undefined, "/tmp/project", async () => {
        createCalls += 1;
        throw new Error("must not create");
      }),
    /capacity 1 is full/,
  );
  assert.equal(createCalls, 0);
  assert.deepEqual(Object.keys(manifest.conversations), ["first"]);
  assert.deepEqual(
    Object.keys(JSON.parse(await readFile(manifestPath, "utf8")).conversations),
    ["first"],
  );
  await first.release();
});

test("an expired same-host lock heartbeat is not stolen from a live PID", async () => {
  const { store, stateDir, config, env } = await setup();
  const sessionFile = await createSessionFile(stateDir, "lock-pid-reuse");
  const active = await store.resolve(
    "thread",
    undefined,
    "/tmp/project",
    async () => ({
      sessionFile,
      piSessionId: "lock-pid-reuse",
      cwd: "/tmp/project",
    }),
  );
  const manifestPath = join(stateDir, "test-agent", "conversations.json");
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  const identity = manifest.conversations.thread.lease;
  await active.release();

  const lockPath = join(stateDir, "test-agent", "locks", "manifest.lock");
  await mkdir(lockPath, { mode: 0o700 });
  await writeFile(
    join(lockPath, "owner"),
    `${JSON.stringify({
      version: 1,
      pid: process.pid,
      token: "reused-pid-token",
      hostId: identity.hostId,
      ...(identity.bootId === undefined ? {} : { bootId: identity.bootId }),
      createdAt: "2000-01-01T00:00:00.000Z",
    })}\n`,
    { mode: 0o600 },
  );
  const old = new Date(Date.now() - 5 * 60_000);
  await utimes(lockPath, old, old);

  const restarted = new ConversationStore(config, silentLogger, env);
  let settled = false;
  const initializing = restarted.initialize().finally(() => {
    settled = true;
  });
  await new Promise((resolve) => setTimeout(resolve, 75));
  assert.equal(
    settled,
    false,
    "a suspended but live local owner must retain its lock",
  );
  await rm(lockPath, { recursive: true, force: true });
  await initializing;
  await assert.rejects(() => stat(lockPath), { code: "ENOENT" });
});

test("manifest and state directory permissions are repaired on initialization", async () => {
  const { store, stateDir } = await setup();
  const sessionFile = await createSessionFile(stateDir, "permissions");
  const active = await store.resolve(
    "permissions",
    undefined,
    "/tmp/project",
    async () => ({
      sessionFile,
      piSessionId: "permissions",
      cwd: "/tmp/project",
    }),
  );
  await active.release();
  const namespaceDir = join(stateDir, "test-agent");
  const manifestPath = join(namespaceDir, "conversations.json");
  assert.equal((await stat(namespaceDir)).mode & 0o777, 0o700);
  assert.equal((await stat(manifestPath)).mode & 0o777, 0o600);
  assert.equal((await stat(sessionFile)).mode & 0o777, 0o600);
});

test("two adapter stores safely serialize simultaneous manifest updates", async () => {
  const { store: left, stateDir, config, env } = await setup();
  const right = new ConversationStore(config, silentLogger, env);
  await Promise.all([left.initialize(), right.initialize()]);
  const leases = await Promise.all(
    Array.from({ length: 24 }, async (_, index) => {
      const store = index % 2 === 0 ? left : right;
      const sessionFile = await createSessionFile(stateDir, `stress-${index}`);
      return store.resolve(
        `stress-${index}`,
        undefined,
        "/tmp/project",
        async () => ({
          sessionFile,
          piSessionId: `stress-${index}`,
          cwd: "/tmp/project",
        }),
      );
    }),
  );
  for (let index = 0; index < 24; index++) {
    assert.equal(
      (await left.get(`stress-${index}`))?.piSessionId,
      `stress-${index}`,
    );
  }
  await Promise.all(leases.map((lease) => lease.release()));
});

test("stale lock cleanup never removes a successor generation after pathname reuse", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-lock-generation-"));
  const lockPath = join(root, "manifest.lock");
  const oldPath = join(root, "manifest.lock.old");
  await mkdir(lockPath);
  await writeFile(join(lockPath, "owner"), "old-generation\n");
  const observed = await captureStateLockGeneration(lockPath);
  assert.ok(observed);

  // Deterministically model the ABA window: the observed lock is released,
  // then a competing owner wins mkdir at the identical pathname before the
  // stale contender attempts its rename/remove.
  await rename(lockPath, oldPath);
  await mkdir(lockPath);
  await writeFile(join(lockPath, "owner"), "successor-generation\n");

  assert.equal(await removeObservedStaleLock(lockPath, observed), false);
  assert.equal(
    await readFile(join(lockPath, "owner"), "utf8"),
    "successor-generation\n",
  );

  const successor = await captureStateLockGeneration(lockPath);
  assert.ok(successor);
  assert.equal(await removeObservedStaleLock(lockPath, successor), true);
  await assert.rejects(() => stat(lockPath), { code: "ENOENT" });
  await rm(root, { recursive: true, force: true });
});
