import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import {
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  stat,
  symlink,
  unlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import {
  SessionManager,
  SettingsManager,
} from "@earendil-works/pi-coding-agent";
import {
  BoundedIpcSendQueue,
  IsolatedPiWorkerFactory,
  PiAgentSessionFactory,
  ReadOnlySettingsStorage,
  applyFreshSessionTitle,
  applyStrictPayloadGuard,
  dedupeCommands,
  estimateProviderContextTokens,
  estimateProviderPayloadTokens,
  guardProviderDispatch,
  installSessionFileQuota,
  requestTimeoutMs,
} from "../dist/index.js";
import { silentLogger, testConfig } from "./helpers.mjs";

const sink = {
  sessionUpdate() {},
  buzzSessionEvent() {},
  usageUpdate() {},
};

test("read-through settings storage discards model and thinking writes byte-for-byte", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-settings-"));
  const cwd = join(root, "workspace");
  const agentDir = join(root, "agent");
  await mkdir(join(cwd, ".pi"), { recursive: true });
  await mkdir(agentDir, { recursive: true });
  const globalPath = join(agentDir, "settings.json");
  const projectPath = join(cwd, ".pi", "settings.json");
  const globalBefore =
    '{\n  "defaultProvider": "anthropic",\n  "defaultModel": "before",\n  "defaultThinkingLevel": "low"\n}\n';
  const projectBefore = '{\n  "theme": "dark"\n}\n';
  await writeFile(globalPath, globalBefore);
  await writeFile(projectPath, projectBefore);

  const manager = SettingsManager.fromStorage(
    new ReadOnlySettingsStorage(cwd, agentDir),
    { projectTrusted: true },
  );
  assert.equal(manager.getDefaultModel(), "before");
  manager.setDefaultModelAndProvider("openai", "after");
  manager.setDefaultThinkingLevel("xhigh");
  manager.setTheme("light");
  await manager.flush();

  assert.equal(await readFile(globalPath, "utf8"), globalBefore);
  assert.equal(await readFile(projectPath, "utf8"), projectBefore);
});

test("fresh titles are bounded and resumed sessions can intentionally skip title replacement", () => {
  const calls = [];
  const session = {
    setSessionName(name) {
      calls.push(name);
    },
  };
  applyFreshSessionTitle(session, `  ${"x".repeat(400)}  `);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].length, 257);
  assert.ok(calls[0].endsWith("…"));
  applyFreshSessionTitle(session, "   ");
  assert.equal(calls.length, 1);
});

test("oversized persisted transcripts fail before Pi opens them", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-cold-quota-"));
  const cwd = join(root, "workspace");
  await mkdir(cwd, { recursive: true });
  const sessionFile = join(root, "oversized.jsonl");
  await writeFile(sessionFile, "x".repeat(4_097));
  const factory = new PiAgentSessionFactory(
    testConfig({ maxSessionFileBytes: 4_096 }),
    silentLogger,
  );
  await assert.rejects(
    () =>
      factory.create({
        cwd,
        persistedSessionFile: sessionFile,
        eventSink: sink,
        acpSessionId: "cold-quota",
      }),
    /BUZZ_SESSION_STORAGE_LIMIT:.*use \/new/,
  );
  assert.equal((await stat(sessionFile)).size, 4_097);
});

test("session transcript quota reserves bounded space for rollback and lifecycle ACK records", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-growth-quota-"));
  const cwd = join(root, "workspace");
  const sessionDir = join(root, "sessions");
  await mkdir(cwd, { recursive: true });
  const manager = SessionManager.create(cwd, sessionDir);
  const pinned = manager;
  pinned._rewriteFile();
  pinned.flushed = true;
  const sessionFile = manager.getSessionFile();
  assert.ok(sessionFile);
  const entry = {
    type: "custom",
    customType: "test.boundary",
    data: { value: "exact" },
    id: "entry-at-boundary",
    parentId: null,
    timestamp: "2026-08-02T00:00:00.000Z",
  };
  const entryBytes = Buffer.byteLength(`${JSON.stringify(entry)}\n`);
  const initialBytes = (await stat(sessionFile)).size;
  const maxBytes = initialBytes + entryBytes + 64 * 1_024;
  installSessionFileQuota(manager, maxBytes);

  pinned._appendEntry({
    type: "custom",
    customType: "buzz.lifecycle_watermark.v1",
    data: { version: 1 },
    id: "lifecycle-watermark-marker",
    parentId: null,
    timestamp: "2026-08-02T00:00:01.000Z",
  });
  pinned._appendEntry({
    type: "custom",
    customType: "buzz.compaction_attempt.v1",
    data: {
      version: 1,
      compactionId: "7f516ff7-e553-4219-b45a-2a432b516cec",
      reason: "threshold",
      beforeTokens: 140_000,
      startedAt: "2026-08-02T00:00:01.500Z",
    },
    id: "compaction-attempt-marker",
    parentId: "lifecycle-watermark-marker",
    timestamp: "2026-08-02T00:00:01.500Z",
  });
  pinned._appendEntry({
    type: "custom",
    customType: "buzz.turn_rollback",
    data: { version: 1 },
    id: "rollback-marker",
    parentId: "compaction-attempt-marker",
    timestamp: "2026-08-02T00:00:02.000Z",
  });

  const deliveryId = "9ba32f72-e8ce-4195-96a2-7b472198bb7e";
  pinned._appendEntry({
    type: "custom",
    customType: "buzz.lifecycle_pending.v1",
    data: {
      version: 1,
      deliveryId,
      sourceEntryId: "compaction-entry",
      event: {
        type: "compaction_completed",
        compactionId: "7f516ff7-e553-4219-b45a-2a432b516cec",
        timestamp: "2026-08-02T00:00:02.500Z",
        message: "Pi compacted this thread's context.",
        piSessionId: "pi-quota-test",
        reason: "threshold",
        beforeTokens: 140_000,
        afterTokens: 30_000,
        limitTokens: 150_000,
        effectiveLimitTokens: 150_000,
        compactionThresholdTokens: 133_616,
        willRetry: false,
        fromExtension: false,
      },
    },
    id: "lifecycle-pending-marker",
    parentId: "rollback-marker",
    timestamp: "2026-08-02T00:00:02.500Z",
  });
  pinned._appendEntry({
    type: "custom",
    customType: "buzz.lifecycle_ack.v1",
    data: { version: 1, deliveryId },
    id: "lifecycle-ack-marker",
    parentId: "lifecycle-pending-marker",
    timestamp: "2026-08-02T00:00:03.000Z",
  });

  // Control records were appended first, yet the ordinary partition still
  // accepts its exact boundary rather than being starved by lifecycle state.
  pinned._appendEntry(entry);
  assert.throws(
    () =>
      pinned._appendEntry({
        ...entry,
        id: "one-byte-too-far",
        parentId: entry.id,
      }),
    /BUZZ_SESSION_STORAGE_LIMIT:.*use \/new/,
  );
  assert.ok((await stat(sessionFile)).size < maxBytes);

  const remainingBytes = maxBytes - (await stat(sessionFile)).size;
  const reserveFiller = (index, paddingLength) => ({
    type: "custom",
    customType: "buzz.turn_rollback",
    data: { version: 1, padding: "x".repeat(paddingLength) },
    id: `reserve-filler-${index}`,
    parentId: index === 0 ? entry.id : `reserve-filler-${index - 1}`,
    timestamp: "2026-08-02T00:00:04.000Z",
  });
  const entryBytesForTarget = (index, targetBytes) => {
    const empty = reserveFiller(index, 0);
    const baseBytes = Buffer.byteLength(`${JSON.stringify(empty)}\n`);
    assert.ok(targetBytes >= baseBytes);
    return reserveFiller(index, targetBytes - baseBytes);
  };
  const fullTargetBytes = 1_800;
  let fullEntries = Math.floor(remainingBytes / fullTargetBytes);
  const remainder = remainingBytes % fullTargetBytes;
  const targets = [];
  if (remainder === 0) {
    targets.push(...Array.from({ length: fullEntries }, () => fullTargetBytes));
  } else {
    const remainderBase = Buffer.byteLength(
      `${JSON.stringify(reserveFiller(fullEntries, 0))}\n`,
    );
    if (remainder >= remainderBase) {
      targets.push(
        ...Array.from({ length: fullEntries }, () => fullTargetBytes),
        remainder,
      );
    } else {
      fullEntries -= 1;
      assert.ok(fullEntries >= 0);
      targets.push(
        ...Array.from({ length: fullEntries }, () => fullTargetBytes),
        fullTargetBytes + remainder,
      );
    }
  }
  for (const [index, targetBytes] of targets.entries()) {
    const filler = entryBytesForTarget(index, targetBytes);
    assert.equal(Buffer.byteLength(`${JSON.stringify(filler)}\n`), targetBytes);
    pinned._appendEntry(filler);
  }
  assert.equal((await stat(sessionFile)).size, maxBytes);
  assert.throws(
    () => pinned._appendEntry(reserveFiller(targets.length, 0)),
    /BUZZ_SESSION_STORAGE_LIMIT:.*use \/new/,
  );
});

test("SIGKILL after Pi compaction append is reconciled and replayed exactly once after ACK", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-compaction-kill-"));
  const cwd = join(root, "workspace");
  const agentDir = join(root, "agent");
  const sessionFile = join(root, "killed-after-compaction.jsonl");
  await mkdir(cwd, { recursive: true });
  await mkdir(agentDir, { recursive: true });

  const childScript = `
    import { writeFileSync } from "node:fs";
    import { SessionManager } from "@earendil-works/pi-coding-agent";
    const [sessionFile, cwd] = process.argv.slice(1);
    writeFileSync(sessionFile, "");
    const manager = SessionManager.open(sessionFile, undefined, cwd);
    manager.appendCustomEntry("buzz.lifecycle_watermark.v1", { version: 1 });
    const firstKeptEntryId = manager.appendCustomEntry("test.seed", { version: 1 });
    manager.appendCompaction(
      "Durable summary written immediately before a forced process death",
      firstKeptEntryId,
      140000,
      undefined,
      false,
    );
    process.kill(process.pid, "SIGKILL");
  `;
  const child = spawn(
    process.execPath,
    ["--input-type=module", "-e", childScript, sessionFile, cwd],
    { cwd: new URL("..", import.meta.url), stdio: ["ignore", "pipe", "pipe"] },
  );
  const [code, signal] = await once(child, "exit");
  assert.equal(code, null);
  assert.equal(signal, "SIGKILL");
  const killedTranscript = await readFile(sessionFile, "utf8");
  assert.match(killedTranscript, /"type":"compaction"/);
  assert.doesNotMatch(killedTranscript, /buzz\.lifecycle_pending\.v1/);

  const previousAgentDir = process.env.PI_CODING_AGENT_DIR;
  process.env.PI_CODING_AGENT_DIR = agentDir;
  try {
    const firstDeliveries = [];
    const factory = new PiAgentSessionFactory(testConfig(), silentLogger);
    const recovered = await factory.create({
      cwd,
      persistedSessionFile: sessionFile,
      eventSink: {
        sessionUpdate() {},
        buzzSessionEvent(_sessionId, event, deliveryId) {
          firstDeliveries.push({ event, deliveryId });
        },
        usageUpdate() {},
      },
      acpSessionId: "compaction-kill-recovery-1",
    });
    await recovered.replayLifecycleEvents();
    assert.equal(firstDeliveries.length, 1);
    assert.match(
      firstDeliveries[0].deliveryId,
      /^[0-9a-f]{8}-[0-9a-f]{4}-5[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u,
    );
    assert.equal(firstDeliveries[0].event.type, "compaction_completed");
    assert.equal(firstDeliveries[0].event.reason, "threshold");
    assert.equal(firstDeliveries[0].event.beforeTokens, 140_000);
    assert.equal(firstDeliveries[0].event.afterTokens, null);
    assert.match(
      firstDeliveries[0].event.message,
      /recovered.*runtime restarted/i,
    );
    assert.match(
      await readFile(sessionFile, "utf8"),
      /buzz\.lifecycle_pending\.v1/,
    );
    await recovered.dispose();

    const retriedDeliveries = [];
    const retried = await factory.create({
      cwd,
      persistedSessionFile: sessionFile,
      eventSink: {
        sessionUpdate() {},
        buzzSessionEvent(_sessionId, event, deliveryId) {
          retriedDeliveries.push({ event, deliveryId });
        },
        usageUpdate() {},
      },
      acpSessionId: "compaction-kill-recovery-2",
    });
    await retried.replayLifecycleEvents();
    assert.deepEqual(retriedDeliveries, firstDeliveries);
    await retried.acknowledgeLifecycleEvent(retriedDeliveries[0].deliveryId);
    await retried.dispose();
    assert.match(
      await readFile(sessionFile, "utf8"),
      /buzz\.lifecycle_ack\.v1/,
    );

    const afterAckDeliveries = [];
    const afterAck = await factory.create({
      cwd,
      persistedSessionFile: sessionFile,
      eventSink: {
        sessionUpdate() {},
        buzzSessionEvent(_sessionId, event, deliveryId) {
          afterAckDeliveries.push({ event, deliveryId });
        },
        usageUpdate() {},
      },
      acpSessionId: "compaction-kill-recovery-3",
    });
    await afterAck.replayLifecycleEvents();
    assert.deepEqual(afterAckDeliveries, []);
    await afterAck.dispose();
  } finally {
    if (previousAgentDir === undefined) delete process.env.PI_CODING_AGENT_DIR;
    else process.env.PI_CODING_AGENT_DIR = previousAgentDir;
  }
});

test("lifecycle recovery honors its feature watermark and only scans the active Pi branch", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-compaction-branch-"));
  const cwd = join(root, "workspace");
  const agentDir = join(root, "agent");
  const sessionDir = join(root, "sessions");
  await mkdir(cwd, { recursive: true });
  await mkdir(agentDir, { recursive: true });
  const manager = SessionManager.create(cwd, sessionDir);
  manager._rewriteFile();
  manager.flushed = true;

  const legacySeed = manager.appendCustomEntry("test.legacy", { version: 1 });
  manager.appendCompaction(
    "Legacy compaction from before Buzz lifecycle delivery existed",
    legacySeed,
    110_000,
    undefined,
    false,
  );
  const watermark = manager.appendCustomEntry("buzz.lifecycle_watermark.v1", {
    version: 1,
  });
  const orphanSeed = manager.appendCustomEntry("test.orphan", { version: 1 });
  manager.appendCompaction(
    "Compaction on a branch that is no longer active",
    orphanSeed,
    120_000,
    undefined,
    false,
  );
  manager.branch(watermark);
  const activeSeed = manager.appendCustomEntry("test.active", { version: 1 });
  manager.appendCompaction(
    "Current branch compaction missing its crash-gap marker",
    activeSeed,
    130_000,
    undefined,
    false,
  );
  const sessionFile = manager.getSessionFile();
  assert.ok(sessionFile);

  const deliveries = [];
  const previousAgentDir = process.env.PI_CODING_AGENT_DIR;
  process.env.PI_CODING_AGENT_DIR = agentDir;
  try {
    const handle = await new PiAgentSessionFactory(
      testConfig(),
      silentLogger,
    ).create({
      cwd,
      persistedSessionFile: sessionFile,
      eventSink: {
        sessionUpdate() {},
        buzzSessionEvent(_sessionId, event, deliveryId) {
          deliveries.push({ event, deliveryId });
        },
        usageUpdate() {},
      },
      acpSessionId: "compaction-active-branch-only",
    });
    await handle.replayLifecycleEvents();
    assert.equal(deliveries.length, 1);
    assert.equal(deliveries[0].event.beforeTokens, 130_000);
    assert.match(deliveries[0].event.message, /recovered.*runtime restarted/i);
    assert.doesNotMatch(
      deliveries[0].event.message,
      /legacy|no longer active/i,
    );
    await handle.acknowledgeLifecycleEvent(deliveries[0].deliveryId);
    await handle.dispose();

    const transcript = await readFile(sessionFile, "utf8");
    assert.equal(transcript.match(/buzz\.lifecycle_pending\.v1/gu)?.length, 1);
  } finally {
    if (previousAgentDir === undefined) delete process.env.PI_CODING_AGENT_DIR;
    else process.env.PI_CODING_AGENT_DIR = previousAgentDir;
  }
});

test("final provider guard includes system prompt, messages, and tool schemas", () => {
  let dispatched = false;
  const context = {
    systemPrompt: "s".repeat(80_000),
    messages: [{ role: "user", content: "hello" }],
    tools: [{ name: "giant", description: "d".repeat(80_000), parameters: {} }],
  };
  assert.ok(estimateProviderContextTokens(context) > 50_000);
  assert.throws(
    () =>
      guardProviderDispatch(context, 50_000, () => {
        dispatched = true;
      }),
    /BUZZ_CONTEXT_LIMIT/,
  );
  assert.equal(
    dispatched,
    false,
    "an oversized request must never reach the provider",
  );
});

test("dense and extension-expanded contexts are rejected before dispatch", () => {
  for (const content of [
    "a ".repeat(60_000),
    "界".repeat(50_000),
    "!".repeat(120_000),
  ]) {
    let dispatched = false;
    assert.throws(
      () =>
        guardProviderDispatch(
          {
            systemPrompt: "",
            messages: [{ role: "user", content }],
            tools: [],
          },
          50_000,
          () => {
            dispatched = true;
          },
        ),
      /BUZZ_CONTEXT_LIMIT/,
    );
    assert.equal(dispatched, false);
  }
});

test("wide custom-provider payloads fail closed before cloning or dispatch", () => {
  const wide = new Array(100_001).fill(0);
  assert.throws(
    () => estimateProviderPayloadTokens({ content: wide }),
    /safe structural bounds/,
  );
  let dispatched = false;
  assert.throws(
    () =>
      guardProviderDispatch(
        {
          systemPrompt: "",
          messages: [{ role: "user", content: wide }],
          tools: [],
        },
        150_000,
        () => {
          dispatched = true;
        },
      ),
    /safe structural bounds/,
  );
  assert.equal(dispatched, false);
});

test("provider payload guard rejects advanced and non-plain JSON values", () => {
  class CustomPayload {
    value = "custom";
  }
  const advancedValues = [
    new ArrayBuffer(16),
    new Uint8Array([1, 2, 3]),
    new DataView(new ArrayBuffer(8)),
    new Map([["key", "value"]]),
    new Set(["value"]),
    new Date("2026-01-01T00:00:00.000Z"),
    new CustomPayload(),
  ];
  if (typeof SharedArrayBuffer !== "undefined") {
    advancedValues.push(new SharedArrayBuffer(16));
  }
  for (const value of advancedValues) {
    assert.throws(
      () => estimateProviderPayloadTokens({ value }),
      /provider payload contains non-plain JSON data/,
    );
  }
  assert.doesNotThrow(() =>
    estimateProviderPayloadTokens(
      Object.assign(Object.create(null), { value: ["plain", 1, true] }),
    ),
  );
});

test("provider payload inspection never executes iterators or accessors", () => {
  let iteratorCalls = 0;
  const overriddenIterator = [{ safe: true }];
  Object.defineProperty(overriddenIterator, Symbol.iterator, {
    value: function* () {
      iteratorCalls += 1;
      while (true) yield { unsafe: true };
    },
  });
  assert.throws(
    () => estimateProviderPayloadTokens(overriddenIterator),
    /symbol-keyed data/,
  );
  assert.equal(iteratorCalls, 0);

  let getterCalls = 0;
  const accessorObject = {};
  Object.defineProperty(accessorObject, "secret", {
    enumerable: true,
    get() {
      getterCalls += 1;
      return "unsafe";
    },
  });
  assert.throws(
    () => estimateProviderPayloadTokens(accessorObject),
    /could not be safely inspected/,
  );
  assert.equal(getterCalls, 0);

  const accessorArray = ["safe"];
  Object.defineProperty(accessorArray, 0, {
    enumerable: true,
    get() {
      getterCalls += 1;
      return "unsafe";
    },
  });
  assert.throws(
    () => estimateProviderPayloadTokens(accessorArray),
    /could not be safely inspected/,
  );
  assert.equal(getterCalls, 0);
});

test("provider payload inspection rejects sparse and decorated arrays", () => {
  const sparse = new Array(2);
  sparse[0] = "first";
  assert.throws(() => estimateProviderPayloadTokens(sparse), /sparse array/);

  const decorated = ["first"];
  decorated.extra = "not provider JSON";
  assert.throws(
    () => estimateProviderPayloadTokens(decorated),
    /decorated array/,
  );

  const symbolObject = { safe: true };
  symbolObject[Symbol("hidden")] = "not provider JSON";
  assert.throws(
    () => estimateProviderPayloadTokens(symbolObject),
    /symbol-keyed data/,
  );
});

test("raw provider payload hooks may observe but cannot mutate or expand requests", async () => {
  const payload = { messages: [{ role: "user", content: "hello" }] };
  assert.equal(
    await applyStrictPayloadGuard(payload, {}, async () => undefined, 10_000),
    payload,
  );
  await assert.rejects(
    () =>
      applyStrictPayloadGuard(
        payload,
        {},
        async (value) => ({ ...value, injected: "x".repeat(100_000) }),
        10_000,
      ),
    /payload mutation is disabled/,
  );
});

test("normal vision payloads budget image tokens without counting base64 as text", async () => {
  const image = Buffer.alloc(1024 * 1024, 0xab).toString("base64");
  const payload = {
    messages: [
      {
        role: "user",
        content: [
          { type: "text", text: "Describe this image" },
          {
            type: "image",
            source: { type: "base64", media_type: "image/png", data: image },
          },
        ],
      },
    ],
  };
  assert.ok(estimateProviderPayloadTokens(payload) < 10_000);
  assert.equal(
    await applyStrictPayloadGuard(payload, {}, async () => undefined, 150_000),
    payload,
  );
  let dispatched = false;
  const piContext = {
    systemPrompt: "Describe images accurately.",
    messages: [
      {
        role: "user",
        content: [
          { type: "text", text: "What is this?" },
          { type: "image", data: image, mimeType: "image/png" },
        ],
      },
    ],
    tools: [],
  };
  guardProviderDispatch(piContext, 10_000, () => {
    dispatched = true;
  });
  assert.equal(dispatched, true);
  assert.ok(
    estimateProviderPayloadTokens({
      contents: [
        { parts: [{ inlineData: { mimeType: "image/png", data: image } }] },
      ],
    }) < 10_000,
  );
});

test("runtime timeouts keep interrupts below the outer Buzz deadline", () => {
  const config = testConfig({
    runtimeRequestTimeoutMs: 10_000,
    runtimeControlTimeoutMs: 2_000,
    runtimeInterruptTimeoutMs: 1_000,
  });
  assert.equal(requestTimeoutMs("prompt", config), 10_000);
  assert.equal(requestTimeoutMs("create", config), 2_000);
  for (const method of ["steer", "abort", "dispose", "shutdown"]) {
    assert.equal(requestTimeoutMs(method, config), 1_000);
  }
});

test("command descriptors deduplicate collisions deterministically", () => {
  assert.deepEqual(
    dedupeCommands([
      { name: "review", description: "extension" },
      { name: "review", description: "prompt" },
      { name: "skill:test", description: "skill" },
    ]),
    [
      { name: "review", description: "extension" },
      { name: "skill:test", description: "skill" },
    ],
  );
});

test("actual Pi SDK creates a titled durable session and resumes its id", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-sdk-"));
  const cwd = join(root, "workspace");
  const agentDir = join(root, "agent");
  await mkdir(cwd, { recursive: true });
  await mkdir(agentDir, { recursive: true });
  const settingsPath = join(agentDir, "settings.json");
  const settingsBefore =
    '{\n  "defaultProvider": "openai",\n  "defaultModel": "gpt-4.1-mini",\n  "defaultThinkingLevel": "low"\n}\n';
  await writeFile(settingsPath, settingsBefore);
  const previousAgentDir = process.env.PI_CODING_AGENT_DIR;
  const previousOpenAiKey = process.env.OPENAI_API_KEY;
  process.env.PI_CODING_AGENT_DIR = agentDir;
  process.env.OPENAI_API_KEY = "buzz-test-key-not-used";
  try {
    const factory = new PiAgentSessionFactory(testConfig(), silentLogger);
    const first = await factory.create({
      cwd,
      title: "Buzz · durable thread",
      eventSink: sink,
      acpSessionId: "sdk-first",
    });
    const firstId = first.piSessionId;
    const sessionFile = first.sessionFile;
    assert.ok(sessionFile);
    assert.match(await readFile(sessionFile, "utf8"), /Buzz · durable thread/);
    const model = first.getModels()[0];
    assert.ok(
      model,
      "a fake environment credential should expose at least one selectable model",
    );
    await first.setModel(model.id);
    await first.setThinkingLevel("high");
    await first.dispose();
    assert.equal(
      await readFile(settingsPath, "utf8"),
      settingsBefore,
      "actual AgentSession model/thinking changes must not persist normal Pi settings",
    );

    const resumed = await factory.create({
      cwd,
      title: "This must not replace the persisted title",
      persistedSessionFile: sessionFile,
      eventSink: sink,
      acpSessionId: "sdk-resumed",
    });
    assert.equal(resumed.piSessionId, firstId);
    const persisted = await readFile(sessionFile, "utf8");
    assert.match(persisted, /Buzz · durable thread/);
    assert.doesNotMatch(persisted, /This must not replace/);
    await resumed.dispose();
  } finally {
    if (previousAgentDir === undefined) delete process.env.PI_CODING_AGENT_DIR;
    else process.env.PI_CODING_AGENT_DIR = previousAgentDir;
    if (previousOpenAiKey === undefined) delete process.env.OPENAI_API_KEY;
    else process.env.OPENAI_API_KEY = previousOpenAiKey;
  }
});

test("an oversized first prompt is refused once without attempting empty compaction", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-oversized-"));
  const cwd = join(root, "workspace");
  const agentDir = join(root, "agent");
  await mkdir(cwd, { recursive: true });
  await mkdir(agentDir, { recursive: true });
  const events = [];
  const eventSink = {
    sessionUpdate() {},
    buzzSessionEvent(_sessionId, event) {
      events.push(event);
    },
    usageUpdate() {},
  };
  const previousAgentDir = process.env.PI_CODING_AGENT_DIR;
  process.env.PI_CODING_AGENT_DIR = agentDir;
  try {
    const handle = await new PiAgentSessionFactory(
      testConfig(),
      silentLogger,
    ).create({
      cwd,
      title: "Oversized prompt test",
      eventSink,
      acpSessionId: "oversized",
    });
    await assert.rejects(
      () => handle.prompt("a ".repeat(150_001)),
      /BUZZ_CONTEXT_LIMIT/,
    );
    assert.equal(events.length, 1);
    assert.equal(events[0].type, "compaction_failed");
    assert.equal(events[0].reason, "preflight");
    assert.equal(events[0].willRetry, false);
    const persisted = await readFile(handle.sessionFile, "utf8");
    assert.doesNotMatch(persisted, /a a a a a/);
    await handle.dispose();
  } finally {
    if (previousAgentDir === undefined) delete process.env.PI_CODING_AGENT_DIR;
    else process.env.PI_CODING_AGENT_DIR = previousAgentDir;
  }
});

test("a fresh threshold-band prompt proceeds without meaningless empty compaction", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-threshold-band-"));
  const cwd = join(root, "workspace");
  const agentDir = join(root, "agent");
  await mkdir(cwd, { recursive: true });
  await mkdir(agentDir, { recursive: true });
  const previousAgentDir = process.env.PI_CODING_AGENT_DIR;
  process.env.PI_CODING_AGENT_DIR = agentDir;
  try {
    const handle = await new PiAgentSessionFactory(
      testConfig(),
      silentLogger,
    ).create({
      cwd,
      eventSink: sink,
      acpSessionId: "threshold-band",
    });
    const state = handle.runtime.session.agent.state;
    const fixedTokens = estimateProviderContextTokens({
      systemPrompt: state.systemPrompt,
      messages: [],
      tools: state.tools,
    });
    const context = handle.getContextSnapshot();
    const incomingTokens =
      context.compactionThresholdTokens - fixedTokens + 1_000;
    assert.ok(incomingTokens > 0);
    assert.ok(fixedTokens + incomingTokens < context.effectiveLimitTokens);
    let compactCalls = 0;
    let promptCalls = 0;
    handle.runtime.session.compact = async () => {
      compactCalls++;
    };
    handle.runtime.session.prompt = async () => {
      promptCalls++;
    };
    assert.equal(await handle.prompt("a ".repeat(incomingTokens)), "end_turn");
    assert.equal(compactCalls, 0);
    assert.equal(promptCalls, 1);
    await handle.dispose();
  } finally {
    if (previousAgentDir === undefined) delete process.env.PI_CODING_AGENT_DIR;
    else process.env.PI_CODING_AGENT_DIR = previousAgentDir;
  }
});

test("manual compact is a successful status no-op on fresh context and never requests Buzz retry", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-manual-compact-"));
  const cwd = join(root, "workspace");
  const agentDir = join(root, "agent");
  await mkdir(cwd, { recursive: true });
  await mkdir(agentDir, { recursive: true });
  const events = [];
  const previousAgentDir = process.env.PI_CODING_AGENT_DIR;
  process.env.PI_CODING_AGENT_DIR = agentDir;
  try {
    const handle = await new PiAgentSessionFactory(
      testConfig(),
      silentLogger,
    ).create({
      cwd,
      eventSink: {
        sessionUpdate() {},
        buzzSessionEvent(_sessionId, event) {
          events.push(event);
        },
        usageUpdate() {},
      },
      acpSessionId: "manual-compact",
    });
    let compactCalls = 0;
    handle.runtime.session.compact = async () => {
      compactCalls++;
      throw new Error("deterministic compaction failure");
    };

    assert.equal(await handle.prompt("/compact"), "end_turn");
    assert.equal(compactCalls, 0);
    assert.equal(events[0].type, "context_status");
    assert.match(events[0].message, /Nothing to compact yet/);

    handle.runtime.session.agent.state.messages.push({
      role: "user",
      content: "existing history",
    });
    assert.equal(await handle.prompt("/compact"), "end_turn");
    assert.equal(compactCalls, 1);
    assert.equal(events[1].type, "compaction_failed");
    assert.equal(events[1].reason, "manual");
    assert.equal(events[1].willRetry, false);
    await handle.dispose();
  } finally {
    if (previousAgentDir === undefined) delete process.env.PI_CODING_AGENT_DIR;
    else process.env.PI_CODING_AGENT_DIR = previousAgentDir;
  }
});

test("cancelled preflight compaction is one terminal context-limit failure", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-preflight-cancel-"));
  const cwd = join(root, "workspace");
  const agentDir = join(root, "agent");
  await mkdir(cwd, { recursive: true });
  await mkdir(agentDir, { recursive: true });
  const events = [];
  const previousAgentDir = process.env.PI_CODING_AGENT_DIR;
  process.env.PI_CODING_AGENT_DIR = agentDir;
  try {
    const handle = await new PiAgentSessionFactory(
      testConfig(),
      silentLogger,
    ).create({
      cwd,
      eventSink: {
        sessionUpdate() {},
        buzzSessionEvent(_sessionId, event) {
          events.push(event);
        },
        usageUpdate() {},
      },
      acpSessionId: "preflight-cancel",
    });
    handle.runtime.session.agent.state.messages.push({
      role: "user",
      content: "old ".repeat(140_000),
    });
    let promptCalls = 0;
    handle.runtime.session.prompt = async () => {
      promptCalls++;
    };
    handle.runtime.session.compact = async () => {
      handle.handleEvent({ type: "compaction_start", reason: "manual" });
      handle.handleEvent({
        type: "compaction_end",
        reason: "manual",
        result: undefined,
        aborted: true,
        willRetry: false,
      });
      throw new Error("Compaction cancelled");
    };

    await assert.rejects(
      () => handle.prompt("continue"),
      /^Error: BUZZ_CONTEXT_LIMIT: preflight compaction could not create safe room/,
    );
    assert.equal(promptCalls, 0);
    assert.equal(events.length, 1);
    assert.equal(events[0].type, "compaction_failed");
    assert.equal(events[0].reason, "preflight");
    assert.equal(events[0].aborted, true);
    assert.equal(events[0].willRetry, false);
    await handle.dispose();
  } finally {
    if (previousAgentDir === undefined) delete process.env.PI_CODING_AGENT_DIR;
    else process.env.PI_CODING_AGENT_DIR = previousAgentDir;
  }
});

test("cancelled turns persist a rollback leaf that excludes partial input after resume", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-cancel-rollback-"));
  const cwd = join(root, "workspace");
  const agentDir = join(root, "agent");
  await mkdir(cwd, { recursive: true });
  await mkdir(agentDir, { recursive: true });
  const previousAgentDir = process.env.PI_CODING_AGENT_DIR;
  process.env.PI_CODING_AGENT_DIR = agentDir;
  try {
    const factory = new PiAgentSessionFactory(testConfig(), silentLogger);
    const handle = await factory.create({
      cwd,
      eventSink: sink,
      acpSessionId: "cancel-rollback",
    });
    const sessionManager = handle.runtime.session.sessionManager;
    handle.runtime.session.prompt = async () => {
      sessionManager.appendCustomMessageEntry(
        "test.cancelled-turn",
        "cancelled secret input",
        false,
      );
      handle.runtime.session.agent.state.messages.push({
        role: "assistant",
        content: [],
        stopReason: "aborted",
      });
    };

    assert.equal(await handle.prompt("cancelled secret input"), "cancelled");
    assert.doesNotMatch(
      JSON.stringify(handle.runtime.session.agent.state.messages),
      /cancelled secret input/,
    );
    const sessionFile = handle.sessionFile;
    assert.match(await readFile(sessionFile, "utf8"), /buzz.turn_rollback/);
    await handle.dispose();

    const resumed = await factory.create({
      cwd,
      persistedSessionFile: sessionFile,
      eventSink: sink,
      acpSessionId: "cancel-rollback-resumed",
    });
    assert.doesNotMatch(
      JSON.stringify(resumed.runtime.session.agent.state.messages),
      /cancelled secret input/,
    );
    await resumed.dispose();
  } finally {
    if (previousAgentDir === undefined) delete process.env.PI_CODING_AGENT_DIR;
    else process.env.PI_CODING_AGENT_DIR = previousAgentDir;
  }
});

test("Pi compaction success/failure events preserve retry classification", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-compaction-events-"));
  const cwd = join(root, "workspace");
  const agentDir = join(root, "agent");
  await mkdir(cwd, { recursive: true });
  await mkdir(agentDir, { recursive: true });
  const events = [];
  const previousAgentDir = process.env.PI_CODING_AGENT_DIR;
  process.env.PI_CODING_AGENT_DIR = agentDir;
  try {
    const handle = await new PiAgentSessionFactory(
      testConfig(),
      silentLogger,
    ).create({
      cwd,
      eventSink: {
        sessionUpdate() {},
        buzzSessionEvent(_sessionId, event) {
          events.push(event);
        },
        usageUpdate() {},
      },
      acpSessionId: "compaction-events",
    });
    handle.handleEvent({ type: "compaction_start", reason: "threshold" });
    handle.handleEvent({
      type: "compaction_end",
      reason: "threshold",
      result: { tokensBefore: 140_000, estimatedTokensAfter: 30_000 },
      aborted: false,
      willRetry: false,
    });
    handle.handleEvent({ type: "compaction_start", reason: "overflow" });
    handle.handleEvent({
      type: "compaction_end",
      reason: "overflow",
      aborted: false,
      errorMessage: "provider failed at /private/path/secret",
      willRetry: true,
    });
    assert.equal(events[0].type, "compaction_completed");
    assert.equal(events[0].willRetry, false);
    assert.equal(events[0].fromExtension, false);
    assert.match(events[0].compactionId, /^[0-9a-f-]{36}$/);
    assert.equal(events[1].type, "compaction_failed");
    assert.equal(events[1].willRetry, true);
    assert.equal(events[1].fromExtension, false);
    assert.doesNotMatch(events[1].error, /private|secret/);
    await handle.dispose();
  } finally {
    if (previousAgentDir === undefined) delete process.env.PI_CODING_AGENT_DIR;
    else process.env.PI_CODING_AGENT_DIR = previousAgentDir;
  }
});

test("tool events normalize advanced values before bounded IPC and truncate by bytes", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-tool-ipc-payload-"));
  const cwd = join(root, "workspace");
  const agentDir = join(root, "agent");
  await mkdir(cwd, { recursive: true });
  await mkdir(agentDir, { recursive: true });
  const updates = [];
  const previousAgentDir = process.env.PI_CODING_AGENT_DIR;
  process.env.PI_CODING_AGENT_DIR = agentDir;
  try {
    class CustomToolValue {
      visible = "kept";
    }
    const handle = await new PiAgentSessionFactory(
      testConfig(),
      silentLogger,
    ).create({
      cwd,
      eventSink: {
        sessionUpdate(_sessionId, update) {
          updates.push(update);
        },
        buzzSessionEvent() {},
        usageUpdate() {},
      },
      acpSessionId: "advanced-tool-ipc",
    });
    const advanced = {
      arrayBuffer: new ArrayBuffer(16),
      view: new Uint16Array([1, 2, 3]),
      map: new Map([["key", { nested: true }]]),
      set: new Set(["first", "second"]),
      date: new Date("2026-01-01T00:00:00.000Z"),
      custom: new CustomToolValue(),
      ...(typeof SharedArrayBuffer === "undefined"
        ? {}
        : { shared: new SharedArrayBuffer(8) }),
    };
    handle.handleEvent({
      type: "tool_execution_start",
      toolCallId: "advanced-input",
      toolName: "extension_tool",
      args: advanced,
    });
    handle.handleEvent({
      type: "tool_execution_end",
      toolCallId: "advanced-output",
      toolName: "extension_tool",
      result: advanced,
      isError: false,
    });
    const rawInput = updates[0].rawInput;
    const rawOutput = updates[1].rawOutput;
    for (const normalized of [rawInput, rawOutput]) {
      assert.equal(normalized.arrayBuffer.$type, "ArrayBuffer");
      assert.equal(normalized.arrayBuffer.byteLength, 16);
      assert.equal(normalized.view.$type, "TypedArray");
      assert.equal(normalized.map.$type, "Map");
      assert.deepEqual(normalized.map.entries, [["key", { nested: true }]]);
      assert.equal(normalized.set.$type, "Set");
      assert.equal(normalized.date.$type, "Date");
      assert.equal(normalized.custom.$type, "NonPlainObject");
      assert.equal(normalized.custom.visible, "kept");
      if (normalized.shared) {
        assert.equal(normalized.shared.$type, "SharedArrayBuffer");
      }
    }

    const sent = [];
    const sender = new BoundedIpcSendQueue(
      (message, callback) => {
        sent.push(message);
        queueMicrotask(() => callback(null));
        return true;
      },
      4,
      256 * 1_024,
      (error) => assert.fail(error.message),
    );
    assert.equal(sender.enqueue(updates[1]), true);
    await new Promise((resolve) => setImmediate(resolve));
    assert.deepEqual(sent, [updates[1]]);

    const wide = {};
    for (let index = 0; index < 20_000; index += 1) {
      wide[`key_${index}`] = index;
    }
    handle.handleEvent({
      type: "tool_execution_update",
      toolCallId: "wide-output",
      toolName: "extension_tool",
      partialResult: wide,
    });
    const normalizedWide = updates[2].rawOutput;
    assert.equal(normalizedWide["[truncated]"], true);
    assert.ok(Object.keys(normalizedWide).length <= 201);

    const inheritedPrototype = {};
    for (let index = 0; index < 1_000; index += 1) {
      inheritedPrototype[`inherited_${index}`] = index;
    }
    const inheritedWide = Object.assign(Object.create(inheritedPrototype), {
      own: "kept",
    });
    handle.handleEvent({
      type: "tool_execution_update",
      toolCallId: "inherited-wide-output",
      toolName: "extension_tool",
      partialResult: inheritedWide,
    });
    const normalizedInherited = updates[3].rawOutput;
    assert.equal(normalizedInherited.$type, "NonPlainObject");
    assert.equal(normalizedInherited.own, "kept");
    assert.equal(normalizedInherited["[truncated]"], true);

    handle.handleEvent({
      type: "tool_execution_end",
      toolCallId: "oversized-output",
      toolName: "extension_tool",
      result: { text: "界".repeat(100_000) },
      isError: false,
    });
    const truncated = updates[4].rawOutput;
    assert.equal(typeof truncated, "string");
    assert.match(truncated, /output truncated by buzz-pi-agent/);
    assert.ok(Buffer.byteLength(truncated) <= 64_000);
    await handle.dispose();
  } finally {
    if (previousAgentDir === undefined) delete process.env.PI_CODING_AGENT_DIR;
    else process.env.PI_CODING_AGENT_DIR = previousAgentDir;
  }
});

test("actual Pi resources load globally and project resources fail closed until trusted", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-resources-"));
  const cwd = join(root, "workspace");
  const agentDir = join(root, "agent");
  await mkdir(join(agentDir, "extensions"), { recursive: true });
  await mkdir(join(agentDir, "prompts"), { recursive: true });
  await mkdir(join(agentDir, "skills", "global-skill"), { recursive: true });
  await mkdir(join(cwd, ".pi", "extensions"), { recursive: true });
  const commandMarker = join(root, "global-command-ran.json");
  await writeFile(
    join(agentDir, "extensions", "global.js"),
    `import { writeFileSync } from "node:fs";
    let moduleCounter = 0;
    export default function (pi) {
      pi.registerCommand("global-command", {
        description: "Global Buzz test command",
        async handler() {
          writeFileSync(${JSON.stringify(commandMarker)}, JSON.stringify({
            moduleCounter: ++moduleCounter,
            tools: pi.getAllTools().map((tool) => tool.name)
          }));
        }
      });
      pi.registerTool({
        name: "global_test_tool",
        label: "Global test tool",
        description: "A no-op integration test tool",
        parameters: { type: "object", properties: {}, additionalProperties: false },
        async execute() { return { content: [{ type: "text", text: "ok" }] }; }
      });
    }`,
  );
  await writeFile(
    join(cwd, ".pi", "extensions", "project.js"),
    `export default function (pi) {
      pi.registerCommand("project-command", {
        description: "Trusted project test command",
        async handler() {}
      });
    }`,
  );
  await writeFile(
    join(agentDir, "prompts", "global-prompt.md"),
    "---\ndescription: Global prompt test\n---\nReview $ARGUMENTS",
  );
  await writeFile(
    join(agentDir, "skills", "global-skill", "SKILL.md"),
    "---\nname: global-skill\ndescription: Global skill test\n---\nDo the skill.",
  );

  const previousAgentDir = process.env.PI_CODING_AGENT_DIR;
  process.env.PI_CODING_AGENT_DIR = agentDir;
  try {
    const untrustedFactory = new PiAgentSessionFactory(
      testConfig({ trustProjectOverride: false }),
      silentLogger,
    );
    const untrusted = await untrustedFactory.create({
      cwd,
      eventSink: sink,
      acpSessionId: "resources-untrusted",
    });
    const untrustedResources = untrusted.getResources();
    assert.equal(untrustedResources.projectTrusted, false);
    assert.equal(untrustedResources.extensions, 1);
    assert.ok(
      untrustedResources.commands.some(
        (command) => command.name === "global-command",
      ),
    );
    assert.ok(
      untrustedResources.commands.some(
        (command) => command.name === "global-prompt",
      ),
    );
    assert.ok(
      untrustedResources.commands.some(
        (command) => command.name === "skill:global-skill",
      ),
    );
    assert.ok(
      !untrustedResources.commands.some(
        (command) => command.name === "project-command",
      ),
    );
    untrusted.runtime.session.agent.state.messages.push({
      role: "assistant",
      content: [],
      stopReason: "error",
      errorMessage: "stale prior error",
    });
    assert.equal(await untrusted.prompt("/global-command"), "end_turn");
    const firstCommand = JSON.parse(await readFile(commandMarker, "utf8"));
    assert.equal(firstCommand.moduleCounter, 1);
    assert.ok(firstCommand.tools.includes("global_test_tool"));
    await untrusted.dispose();

    const trustedFactory = new PiAgentSessionFactory(
      testConfig({ trustProjectOverride: true }),
      silentLogger,
    );
    const trusted = await trustedFactory.create({
      cwd,
      eventSink: sink,
      acpSessionId: "resources-trusted",
    });
    const trustedResources = trusted.getResources();
    assert.equal(trustedResources.projectTrusted, true);
    assert.equal(trustedResources.extensions, 2);
    assert.ok(
      trustedResources.commands.some(
        (command) => command.name === "project-command",
      ),
    );
    assert.equal(await trusted.prompt("/global-command"), "end_turn");
    const secondCommand = JSON.parse(await readFile(commandMarker, "utf8"));
    assert.equal(
      secondCommand.moduleCounter,
      2,
      "Pi module globals are shared while factory-created handlers remain per session",
    );
    await trusted.dispose();
  } finally {
    if (previousAgentDir === undefined) delete process.env.PI_CODING_AGENT_DIR;
    else process.env.PI_CODING_AGENT_DIR = previousAgentDir;
  }
});

test("extension-only configured models resolve on cold create and cold restore", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-custom-provider-"));
  const cwd = join(root, "workspace");
  const agentDir = join(root, "agent");
  await mkdir(cwd, { recursive: true });
  await mkdir(join(agentDir, "extensions"), { recursive: true });
  await writeFile(
    join(agentDir, "settings.json"),
    JSON.stringify({
      defaultProvider: "buzz-cold-provider",
      defaultModel: "buzz-cold-model",
      defaultThinkingLevel: "low",
    }),
  );
  await writeFile(
    join(agentDir, "extensions", "cold-provider.js"),
    `export default function (pi) {
      pi.registerProvider("buzz-cold-provider", {
        name: "Buzz cold provider",
        baseUrl: "https://cold.invalid/v1",
        apiKey: "test-key-not-used",
        api: "openai-completions",
        models: [{
          id: "buzz-cold-model",
          name: "Buzz cold model",
          reasoning: false,
          input: ["text"],
          cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
          contextWindow: 200000,
          maxTokens: 4096
        }]
      });
    }`,
  );

  const previousAgentDir = process.env.PI_CODING_AGENT_DIR;
  process.env.PI_CODING_AGENT_DIR = agentDir;
  try {
    const initial = await new PiAgentSessionFactory(
      testConfig(),
      silentLogger,
    ).create({
      cwd,
      eventSink: sink,
      acpSessionId: "custom-provider-initial",
    });
    assert.equal(
      initial.getContextSnapshot().model,
      "buzz-cold-provider/buzz-cold-model",
      "provider registrations must be flushed before initial model resolution",
    );
    await initial.setModel("buzz-cold-provider/buzz-cold-model");
    initial.runtime.session.sessionManager.appendCustomMessageEntry(
      "buzz.test.history",
      "persisted custom provider history",
      false,
    );
    const sessionFile = initial.sessionFile;
    assert.ok(sessionFile);
    await initial.dispose();

    const restored = await new PiAgentSessionFactory(
      testConfig(),
      silentLogger,
    ).create({
      cwd,
      persistedSessionFile: sessionFile,
      eventSink: sink,
      acpSessionId: "custom-provider-restored",
    });
    assert.equal(
      restored.getContextSnapshot().model,
      "buzz-cold-provider/buzz-cold-model",
      "a fresh runtime must resolve the extension model saved in the session",
    );
    assert.match(
      JSON.stringify(restored.runtime.session.agent.state.messages),
      /persisted custom provider history/,
    );
    await restored.dispose();
  } finally {
    if (previousAgentDir === undefined) delete process.env.PI_CODING_AGENT_DIR;
    else process.env.PI_CODING_AGENT_DIR = previousAgentDir;
  }
});

test("invalid extension provider registrations surface bounded safe Buzz diagnostics", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-broken-provider-"));
  const cwd = join(root, "workspace");
  const agentDir = join(root, "agent");
  const extensionPath = join(
    agentDir,
    "extensions",
    "broken-provider-secret.js",
  );
  await mkdir(cwd, { recursive: true });
  await mkdir(join(agentDir, "extensions"), { recursive: true });
  await writeFile(
    extensionPath,
    `export default function (pi) {
      pi.registerProvider("buzz-broken-provider", {
        models: [{
          id: "broken-model",
          name: "Broken model",
          reasoning: false,
          input: ["text"],
          cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
          contextWindow: 200000,
          maxTokens: 4096
        }]
      });
    }`,
  );

  const lifecycleEvents = [];
  const previousAgentDir = process.env.PI_CODING_AGENT_DIR;
  process.env.PI_CODING_AGENT_DIR = agentDir;
  try {
    const handle = await new PiAgentSessionFactory(
      testConfig(),
      silentLogger,
    ).create({
      cwd,
      eventSink: {
        sessionUpdate() {},
        buzzSessionEvent(_sessionId, event) {
          lifecycleEvents.push(event);
        },
        usageUpdate() {},
      },
      acpSessionId: "broken-provider-diagnostic",
    });
    const diagnostic = handle
      .getResources()
      .errors.find((error) => error.startsWith("Pi runtime error:"));
    assert.ok(diagnostic, "provider setup diagnostics must reach Buzz");
    assert.match(diagnostic, /buzz-broken-provider|baseUrl|api/i);
    assert.ok(diagnostic.length <= 512);
    assert.doesNotMatch(diagnostic, new RegExp(root));
    assert.doesNotMatch(diagnostic, /broken-provider-secret\.js/);

    assert.equal(await handle.prompt("/reload"), "end_turn");
    const reload = lifecycleEvents.find(
      (event) => event.type === "extensions_reloaded",
    );
    assert.ok(reload);
    assert.ok(
      reload.errors.some((error) => error.startsWith("Pi runtime error:")),
      "reload lifecycle reporting must retain provider setup diagnostics",
    );
    assert.ok(reload.errors.length <= 20);
    assert.ok(reload.errors.every((error) => error.length <= 512));
    await handle.dispose();
  } finally {
    if (previousAgentDir === undefined) delete process.env.PI_CODING_AGENT_DIR;
    else process.env.PI_CODING_AGENT_DIR = previousAgentDir;
  }
});

test("project providers with the same id stay isolated across cwd sessions", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-provider-isolation-"));
  const agentDir = join(root, "agent");
  const cwdA = join(root, "workspace-a");
  const cwdB = join(root, "workspace-b");
  await mkdir(agentDir, { recursive: true });

  for (const [cwd, modelName, baseUrl] of [
    [cwdA, "Workspace A model", "https://workspace-a.invalid/v1"],
    [cwdB, "Workspace B model", "https://workspace-b.invalid/v1"],
  ]) {
    await mkdir(join(cwd, ".pi", "extensions"), { recursive: true });
    await writeFile(
      join(cwd, ".pi", "extensions", "provider.js"),
      `export default function (pi) {
        pi.registerProvider("shared-project-provider", {
          name: "Shared project provider",
          baseUrl: ${JSON.stringify(baseUrl)},
          apiKey: "test-key-not-used",
          api: "openai-completions",
          models: [{
            id: "same-model-id",
            name: ${JSON.stringify(modelName)},
            reasoning: false,
            input: ["text"],
            cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
            contextWindow: 200000,
            maxTokens: 4096
          }]
        });
      }`,
    );
  }

  const previousAgentDir = process.env.PI_CODING_AGENT_DIR;
  process.env.PI_CODING_AGENT_DIR = agentDir;
  try {
    const factory = new PiAgentSessionFactory(
      testConfig({ trustProjectOverride: true }),
      silentLogger,
    );
    const sessionA = await factory.create({
      cwd: cwdA,
      eventSink: sink,
      acpSessionId: "provider-isolation-a",
    });
    const sessionB = await factory.create({
      cwd: cwdB,
      eventSink: sink,
      acpSessionId: "provider-isolation-b",
    });
    const modelId = "shared-project-provider/same-model-id";
    assert.equal(
      sessionA.getModels().find((model) => model.id === modelId)?.name,
      "Workspace A model",
    );
    assert.equal(
      sessionB.getModels().find((model) => model.id === modelId)?.name,
      "Workspace B model",
    );
    assert.notEqual(
      sessionA.runtime.services.modelRuntime,
      sessionB.runtime.services.modelRuntime,
      "each cwd-bound Pi session must own its provider registry",
    );
    await sessionA.dispose();
    await sessionB.dispose();
  } finally {
    if (previousAgentDir === undefined) delete process.env.PI_CODING_AGENT_DIR;
    else process.env.PI_CODING_AGENT_DIR = previousAgentDir;
  }
});

test("project trust revocation removes project providers and preserves global providers", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-provider-revocation-"));
  const agentDir = join(root, "agent");
  const cwd = join(root, "workspace");
  await mkdir(join(agentDir, "extensions"), { recursive: true });
  await mkdir(join(cwd, ".pi", "extensions"), { recursive: true });

  const providerSource = (providerId, modelId, commandName) =>
    `export default function (pi) {
      pi.registerProvider(${JSON.stringify(providerId)}, {
        name: ${JSON.stringify(providerId)},
        baseUrl: "https://provider.invalid/v1",
        apiKey: "test-key-not-used",
        api: "openai-completions",
        models: [{
          id: ${JSON.stringify(modelId)},
          name: ${JSON.stringify(modelId)},
          reasoning: false,
          input: ["text"],
          cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
          contextWindow: 200000,
          maxTokens: 4096
        }]
      });
      pi.registerCommand(${JSON.stringify(commandName)}, {
        description: ${JSON.stringify(commandName)},
        async handler() {}
      });
    }`;

  await writeFile(
    join(agentDir, "extensions", "global-provider.js"),
    providerSource(
      "global-reload-provider",
      "global-reload-model",
      "global-reload-command",
    ),
  );
  await writeFile(
    join(cwd, ".pi", "extensions", "project-provider.js"),
    providerSource(
      "project-reload-provider",
      "project-reload-model",
      "project-reload-command",
    ),
  );

  const canonicalCwd = await realpath(cwd);
  const trustPath = join(agentDir, "trust.json");
  await writeFile(trustPath, JSON.stringify({ [canonicalCwd]: true }));
  const previousAgentDir = process.env.PI_CODING_AGENT_DIR;
  process.env.PI_CODING_AGENT_DIR = agentDir;
  try {
    const handle = await new PiAgentSessionFactory(
      testConfig(),
      silentLogger,
    ).create({
      cwd,
      eventSink: sink,
      acpSessionId: "provider-trust-revocation",
    });
    const projectModelId = "project-reload-provider/project-reload-model";
    const globalModelId = "global-reload-provider/global-reload-model";
    assert.equal(handle.getResources().projectTrusted, true);
    assert.ok(handle.getModels().some((model) => model.id === projectModelId));
    assert.ok(handle.getModels().some((model) => model.id === globalModelId));
    await handle.setModel(projectModelId);
    assert.equal(handle.getContextSnapshot().model, projectModelId);

    await writeFile(trustPath, JSON.stringify({ [canonicalCwd]: false }));
    const resources = await handle.reload();

    assert.equal(resources.projectTrusted, false);
    assert.ok(
      !resources.commands.some(
        (command) => command.name === "project-reload-command",
      ),
    );
    assert.ok(
      resources.commands.some(
        (command) => command.name === "global-reload-command",
      ),
    );
    assert.ok(!handle.getModels().some((model) => model.id === projectModelId));
    assert.ok(handle.getModels().some((model) => model.id === globalModelId));
    assert.equal(
      handle.getContextSnapshot().model,
      null,
      "the revoked provider must not remain active for one more dispatch",
    );
    await assert.rejects(
      () => handle.setModel(projectModelId),
      /Unknown or unavailable Pi model/,
    );
    await handle.dispose();
  } finally {
    if (previousAgentDir === undefined) delete process.env.PI_CODING_AGENT_DIR;
    else process.env.PI_CODING_AGENT_DIR = previousAgentDir;
  }
});

test("a trusted workspace symlink cannot reload onto an untrusted target", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-workspace-symlink-"));
  const agentDir = join(root, "agent");
  const workspaceA = join(root, "trusted-a");
  const workspaceB = join(root, "untrusted-b");
  const workspaceAlias = join(root, "workspace-current");
  const untrustedMarker = join(root, "untrusted-command-ran");
  await mkdir(agentDir, { recursive: true });
  await mkdir(join(workspaceA, ".pi", "extensions"), { recursive: true });
  await mkdir(join(workspaceB, ".pi", "extensions"), { recursive: true });
  await writeFile(
    join(workspaceA, ".pi", "extensions", "trusted.js"),
    `export default function (pi) {
      pi.registerCommand("trusted-only", { description: "trusted", async handler() {} });
    }`,
  );
  await writeFile(
    join(workspaceB, ".pi", "extensions", "untrusted.js"),
    `import { writeFileSync } from "node:fs";
    export default function (pi) {
      pi.registerCommand("untrusted-only", {
        description: "untrusted",
        async handler() { writeFileSync(${JSON.stringify(untrustedMarker)}, "ran"); }
      });
    }`,
  );
  await symlink(workspaceA, workspaceAlias, "dir");
  const canonicalA = await realpath(workspaceA);
  const canonicalB = await realpath(workspaceB);
  await writeFile(
    join(agentDir, "trust.json"),
    JSON.stringify({ [canonicalA]: true, [canonicalB]: false }),
  );

  const previousAgentDir = process.env.PI_CODING_AGENT_DIR;
  const previousHome = process.env.HOME;
  process.env.PI_CODING_AGENT_DIR = agentDir;
  process.env.HOME = root;
  try {
    const handle = await new PiAgentSessionFactory(
      testConfig(),
      silentLogger,
    ).create({
      cwd: workspaceAlias,
      eventSink: sink,
      acpSessionId: "workspace-symlink-swap",
    });
    assert.equal(handle.cwd, canonicalA);
    assert.ok(
      handle
        .getResources()
        .commands.some((command) => command.name === "trusted-only"),
    );

    await unlink(workspaceAlias);
    await symlink(workspaceB, workspaceAlias, "dir");
    await assert.rejects(
      () => handle.reload(),
      /BUZZ_PI_SESSION_INVALIDATED:.*BUZZ_PI_WORKSPACE_CHANGED/,
    );
    assert.equal(handle.isValid, false);
    await assert.rejects(
      () => handle.prompt("/untrusted-only"),
      /BUZZ_PI_SESSION_INVALIDATED/,
    );
    await assert.rejects(() => readFile(untrustedMarker), /ENOENT/);
    await handle.dispose();
  } finally {
    if (previousAgentDir === undefined) delete process.env.PI_CODING_AGENT_DIR;
    else process.env.PI_CODING_AGENT_DIR = previousAgentDir;
    if (previousHome === undefined) delete process.env.HOME;
    else process.env.HOME = previousHome;
  }
});

test("resource mutation during actual SDK reload retires the old host session and permits a fresh one", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-resource-toctou-"));
  const cwd = join(root, "workspace");
  const agentDir = join(root, "agent");
  const promptPath = join(agentDir, "prompts", "stable.md");
  const triggerPath = join(root, "mutate-on-import");
  await mkdir(cwd, { recursive: true });
  await mkdir(join(agentDir, "extensions"), { recursive: true });
  await mkdir(join(agentDir, "prompts"), { recursive: true });
  await writeFile(promptPath, "---\ndescription: Stable prompt\n---\nstable");
  await writeFile(
    join(agentDir, "extensions", "mutation.js"),
    `import { appendFileSync, existsSync } from "node:fs";
    if (existsSync(${JSON.stringify(triggerPath)})) {
      appendFileSync(${JSON.stringify(promptPath)}, "\\nmutated-during-sdk-load");
    }
    export default function (pi) {
      pi.registerCommand("stable-command", {
        description: "Stable command",
        async handler() {}
      });
    }`,
  );

  const previousAgentDir = process.env.PI_CODING_AGENT_DIR;
  const previousHome = process.env.HOME;
  process.env.PI_CODING_AGENT_DIR = agentDir;
  process.env.HOME = root;
  const factory = new IsolatedPiWorkerFactory(testConfig(), silentLogger);
  try {
    const options = {
      cwd,
      eventSink: sink,
      acpSessionId: "resource-toctou-generation",
    };
    const old = await factory.create(options);
    assert.ok(
      old
        .getResources()
        .commands.some((command) => command.name === "stable-command"),
    );
    await writeFile(triggerPath, "1");
    await assert.rejects(
      () => old.reload(),
      /BUZZ_PI_SESSION_INVALIDATED:.*BUZZ_PI_RESOURCE_CHANGED/,
    );
    assert.equal(old.isValid, false);
    await assert.rejects(
      () => old.prompt("old generation must not run"),
      /BUZZ_PI_SESSION_INVALIDATED/,
    );

    await unlink(triggerPath);
    await writeFile(promptPath, "---\ndescription: Stable prompt\n---\nstable");
    const fresh = await factory.create(options);
    assert.equal(fresh.isValid, true);
    assert.equal(await fresh.prompt("/stable-command"), "end_turn");
    await fresh.dispose();
    await old.dispose();
  } finally {
    await factory.shutdown().catch(() => {});
    if (previousAgentDir === undefined) delete process.env.PI_CODING_AGENT_DIR;
    else process.env.PI_CODING_AGENT_DIR = previousAgentDir;
    if (previousHome === undefined) delete process.env.HOME;
    else process.env.HOME = previousHome;
  }
});

test("an actual extension command can reload resources while its prompt is active", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-command-reload-"));
  const cwd = join(root, "workspace");
  const agentDir = join(root, "agent");
  const marker = join(root, "reload-command.txt");
  const reloadedPrompt = join(agentDir, "prompts", "created-on-reload.md");
  await mkdir(cwd, { recursive: true });
  await mkdir(join(agentDir, "extensions"), { recursive: true });
  await mkdir(join(agentDir, "prompts"), { recursive: true });
  await writeFile(
    join(agentDir, "extensions", "reload-command.js"),
    `import { appendFileSync, writeFileSync } from "node:fs";
    export default function (pi) {
      pi.registerCommand("reload-self", {
        description: "Reload from inside an extension command",
        async handler(_args, ctx) {
          appendFileSync(${JSON.stringify(marker)}, "before\\n");
          writeFileSync(${JSON.stringify(reloadedPrompt)}, "---\\ndescription: Created during reload\\n---\\nNew resource");
          await ctx.reload();
          appendFileSync(${JSON.stringify(marker)}, "after\\n");
        }
      });
    }`,
  );
  const lifecycleEvents = [];
  const sessionUpdates = [];
  const previousAgentDir = process.env.PI_CODING_AGENT_DIR;
  process.env.PI_CODING_AGENT_DIR = agentDir;
  try {
    const handle = await new PiAgentSessionFactory(
      testConfig(),
      silentLogger,
    ).create({
      cwd,
      eventSink: {
        sessionUpdate(_sessionId, update) {
          sessionUpdates.push(update);
        },
        buzzSessionEvent(_sessionId, event) {
          lifecycleEvents.push(event);
        },
        usageUpdate() {},
      },
      acpSessionId: "extension-command-reload",
    });
    assert.ok(
      handle
        .getResources()
        .commands.some((command) => command.name === "reload-self"),
    );
    assert.equal(await handle.prompt("/reload-self"), "end_turn");
    assert.equal(await readFile(marker, "utf8"), "before\nafter\n");
    assert.equal(handle.getResources().prompts, 1);
    assert.ok(
      lifecycleEvents.some(
        (event) => event.type === "extensions_reloaded" && event.prompts === 1,
      ),
      "command-context reload should report refreshed resources to Buzz",
    );
    assert.ok(
      sessionUpdates.some(
        (update) => update.sessionUpdate === "available_commands_update",
      ),
    );
    assert.ok(
      handle
        .getResources()
        .commands.some((command) => command.name === "reload-self"),
      "the refreshed extension command registry should remain active",
    );
    await handle.dispose();
  } finally {
    if (previousAgentDir === undefined) delete process.env.PI_CODING_AGENT_DIR;
    else process.env.PI_CODING_AGENT_DIR = previousAgentDir;
  }
});

test("external resource reload remains blocked during an ordinary active turn", async () => {
  const root = await mkdtemp(join(tmpdir(), "buzz-pi-external-reload-busy-"));
  const cwd = join(root, "workspace");
  const agentDir = join(root, "agent");
  await mkdir(cwd, { recursive: true });
  await mkdir(agentDir, { recursive: true });
  const previousAgentDir = process.env.PI_CODING_AGENT_DIR;
  process.env.PI_CODING_AGENT_DIR = agentDir;
  try {
    const handle = await new PiAgentSessionFactory(
      testConfig(),
      silentLogger,
    ).create({
      cwd,
      eventSink: sink,
      acpSessionId: "external-reload-busy",
    });
    let markStarted;
    let finishPrompt;
    const started = new Promise((resolve) => {
      markStarted = resolve;
    });
    const blocked = new Promise((resolve) => {
      finishPrompt = resolve;
    });
    handle.runtime.session.prompt = async () => {
      markStarted();
      await blocked;
    };

    const turn = handle.prompt("ordinary turn");
    await started;
    await assert.rejects(
      () => handle.reload(),
      /cannot reload resources while a turn is active/,
    );
    finishPrompt();
    assert.equal(await turn, "end_turn");
    await handle.dispose();
  } finally {
    if (previousAgentDir === undefined) delete process.env.PI_CODING_AGENT_DIR;
    else process.env.PI_CODING_AGENT_DIR = previousAgentDir;
  }
});
