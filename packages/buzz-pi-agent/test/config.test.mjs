import assert from "node:assert/strict";
import { test } from "node:test";
import {
  compactionThreshold,
  effectiveCompactionSettings,
  effectiveContextLimit,
  loadConfig,
  logicalModelContextWindow,
} from "../dist/index.js";

test("150k is a logical safety ceiling with a separate model-safe compaction threshold", () => {
  assert.equal(effectiveContextLimit(200_000, 150_000), 150_000);
  assert.equal(logicalModelContextWindow(200_000, 150_000), 150_000);
  assert.equal(compactionThreshold(150_000, 16_384), 133_616);
});

test("models with smaller windows compact below their own ceiling", () => {
  assert.equal(effectiveContextLimit(128_000, 150_000), 128_000);
  assert.equal(logicalModelContextWindow(128_000, 150_000), 128_000);
  assert.equal(compactionThreshold(128_000, 16_384), 111_616);
  assert.deepEqual(effectiveCompactionSettings(32_000, 16_384, 24_000), {
    reserveTokens: 8_000,
    keepRecentTokens: 18_000,
    thresholdTokens: 24_000,
  });
});

test("configuration defaults to 150k, bounded persistence, and fail-closed trust override", () => {
  const config = loadConfig({ HOME: "/tmp/pi-home" });
  assert.equal(config.contextLimitTokens, 150_000);
  assert.equal(config.maxSessions, 12);
  assert.equal(config.sessionTtlMs, 45 * 60 * 1_000);
  assert.equal(config.maxPersistedConversations, 512);
  assert.equal(config.persistedConversationTtlMs, 90 * 24 * 60 * 60 * 1_000);
  assert.equal(config.maxPendingResetTombstones, 512);
  assert.equal(config.maxRetainedResetTombstones, 512);
  assert.equal(config.resetTombstoneTtlMs, 30 * 24 * 60 * 60 * 1_000);
  assert.equal(config.maxSessionFileBytes, 64 * 1_024 * 1_024);
  assert.equal(config.maxPendingSessionEvents, 256);
  assert.equal(config.trustProjectOverride, undefined);
  assert.equal(config.runtimeRequestTimeoutMs, 110 * 60 * 1_000);
  assert.equal(config.runtimeInterruptTimeoutMs, 1_200);
  assert.equal(config.maxOutputQueueMessages, 2_048);
  assert.equal(config.maxActiveRequests, 64);
  assert.equal(config.maxOutputQueueBytes, 16 * 1_024 * 1_024);
  assert.equal(config.maxRuntimeIpcQueueMessages, 1_024);
  assert.equal(config.maxRuntimeIpcQueueBytes, 16 * 1_024 * 1_024);
  assert.equal(config.maxResourceFileBytes, 1 * 1_024 * 1_024);
  assert.equal(config.maxResourceTotalBytes, 16 * 1_024 * 1_024);
  assert.equal(config.maxResourceFiles, 512);
  assert.equal(config.maxResourceEntries, 4_096);
  assert.equal(config.maxResourceDepth, 16);
});

test("invalid numeric and trust settings fail startup", () => {
  assert.throws(
    () => loadConfig({ BUZZ_PI_CONTEXT_LIMIT: "0" }),
    /positive integer/,
  );
  assert.throws(
    () => loadConfig({ BUZZ_PI_TRUST_PROJECT: "maybe" }),
    /boolean/,
  );
  assert.throws(
    () =>
      loadConfig({
        BUZZ_PI_CONTEXT_LIMIT: "100",
        BUZZ_PI_COMPACTION_RESERVE: "100",
        BUZZ_PI_KEEP_RECENT_TOKENS: "1",
      }),
    /COMPACTION_RESERVE/,
  );
  assert.throws(
    () =>
      loadConfig({
        BUZZ_PI_CONTEXT_LIMIT: "100",
        BUZZ_PI_COMPACTION_RESERVE: "10",
        BUZZ_PI_KEEP_RECENT_TOKENS: "90",
      }),
    /KEEP_RECENT/,
  );
  assert.throws(
    () => loadConfig({ BUZZ_PI_RUNTIME_INTERRUPT_TIMEOUT_MS: "120000" }),
    /INTERRUPT_TIMEOUT/,
  );
  assert.throws(
    () => loadConfig({ BUZZ_PI_MAX_PENDING_RESET_TOMBSTONES: "513" }),
    /must not exceed 512/,
  );
  assert.throws(
    () => loadConfig({ BUZZ_PI_MAX_RETAINED_RESET_TOMBSTONES: "513" }),
    /must not exceed 512/,
  );
  assert.throws(
    () => loadConfig({ BUZZ_PI_RESET_TOMBSTONE_TTL_MS: "31536000001" }),
    /must not exceed 31536000000/,
  );
  assert.throws(
    () => loadConfig({ BUZZ_PI_MAX_OUTPUT_QUEUE_MESSAGES: "8193" }),
    /must not exceed 8192/,
  );
  assert.throws(
    () => loadConfig({ BUZZ_PI_MAX_RUNTIME_IPC_QUEUE_MESSAGES: "4097" }),
    /must not exceed 4096/,
  );
  assert.throws(
    () => loadConfig({ BUZZ_PI_MAX_ACTIVE_REQUESTS: "513" }),
    /must not exceed 512/,
  );
  assert.throws(
    () => loadConfig({ BUZZ_PI_MAX_PERSISTED_CONVERSATIONS: "513" }),
    /must not exceed 512/,
  );
  assert.throws(
    () => loadConfig({ BUZZ_PI_MAX_SESSION_FILE_BYTES: "1048575" }),
    /must be at least 1048576/,
  );
  assert.throws(
    () => loadConfig({ BUZZ_PI_MAX_PENDING_SESSION_EVENTS: "513" }),
    /must not exceed 512/,
  );
  assert.throws(
    () =>
      loadConfig({
        BUZZ_PI_MAX_RESOURCE_FILE_BYTES: "1024",
        BUZZ_PI_MAX_RESOURCE_TOTAL_BYTES: "512",
      }),
    /TOTAL_BYTES must be at least/,
  );
  assert.throws(
    () =>
      loadConfig({
        BUZZ_PI_MAX_RESOURCE_FILES: "10",
        BUZZ_PI_MAX_RESOURCE_ENTRIES: "9",
      }),
    /ENTRIES must be at least/,
  );
});

test("every resource and timer setting has a production upper bound", () => {
  for (const [name, maximum] of [
    ["BUZZ_PI_CONTEXT_LIMIT", 150_000],
    ["BUZZ_PI_COMPACTION_RESERVE", 149_999],
    ["BUZZ_PI_KEEP_RECENT_TOKENS", 149_999],
    ["BUZZ_PI_MAX_SESSIONS", 128],
    ["BUZZ_PI_SESSION_TTL_MS", 24 * 60 * 60 * 1_000],
    ["BUZZ_PI_SWEEP_INTERVAL_MS", 60 * 60 * 1_000],
    ["BUZZ_PI_MAX_LINE_BYTES", 64 * 1_024 * 1_024],
    ["BUZZ_PI_PERSISTED_CONVERSATION_TTL_MS", 365 * 24 * 60 * 60 * 1_000],
    ["BUZZ_PI_CONVERSATION_LEASE_MS", 24 * 60 * 60 * 1_000],
    ["BUZZ_PI_MAX_SESSION_FILE_BYTES", 512 * 1_024 * 1_024],
    ["BUZZ_PI_MAX_PENDING_SESSION_EVENTS", 512],
    ["BUZZ_PI_RUNTIME_REQUEST_TIMEOUT_MS", 6 * 60 * 60 * 1_000],
    ["BUZZ_PI_RUNTIME_CONTROL_TIMEOUT_MS", 30 * 60 * 1_000],
    ["BUZZ_PI_RUNTIME_INTERRUPT_TIMEOUT_MS", 60 * 1_000],
    ["BUZZ_PI_MAX_RESOURCE_FILE_BYTES", 16 * 1_024 * 1_024],
    ["BUZZ_PI_MAX_RESOURCE_TOTAL_BYTES", 64 * 1_024 * 1_024],
    ["BUZZ_PI_MAX_RESOURCE_FILES", 4_096],
    ["BUZZ_PI_MAX_RESOURCE_ENTRIES", 16_384],
    ["BUZZ_PI_MAX_RESOURCE_DEPTH", 64],
  ]) {
    assert.throws(
      () => loadConfig({ [name]: String(maximum + 1) }),
      new RegExp(`${name} must not exceed ${maximum}`),
      name,
    );
  }
});

test("absurdly small custom model windows are rejected", () => {
  assert.throws(() => effectiveCompactionSettings(7, 2, 2), /too small/);
});

test("maximum retention bounds fit below the durable manifest write ceiling", () => {
  const conversations = {};
  const resetTombstones = {};
  const timestamp = "2026-08-02T00:00:00.000Z";
  const escaped = '"';
  for (let index = 0; index < 512; index += 1) {
    const conversationId = `c${index}`.padEnd(512, escaped);
    const piSessionId = escaped.repeat(256);
    const resetToken = escaped.repeat(512);
    conversations[conversationId] = {
      conversationId,
      cwd: `/${escaped.repeat(2_047)}`,
      sessionFile: `/${escaped.repeat(4_089)}.jsonl`,
      piSessionId,
      createdAt: timestamp,
      lastUsedAt: timestamp,
      lastResetToken: resetToken,
      relayHistoryCleared: true,
      lease: {
        ownerId: escaped.repeat(256),
        pid: 999_999,
        hostId: escaped.repeat(128),
        bootId: escaped.repeat(128),
        expiresAt: timestamp,
      },
    };
    resetTombstones[conversationId] = {
      conversationId,
      previousPiSessionId: piSessionId,
      resetToken,
      createdAt: timestamp,
      status: "retained",
      installedPiSessionId: piSessionId,
      consumedAt: timestamp,
    };
  }
  for (let index = 0; index < 512; index += 1) {
    const conversationId = `p${index}`.padEnd(512, escaped);
    resetTombstones[conversationId] = {
      conversationId,
      previousPiSessionId: escaped.repeat(256),
      resetToken: escaped.repeat(512),
      createdAt: timestamp,
      status: "pending",
    };
  }
  const bytes = Buffer.byteLength(
    `${JSON.stringify({ version: 1, conversations, resetTombstones }, null, 2)}\n`,
  );
  assert.ok(bytes < 16 * 1_024 * 1_024, `${bytes} must fit below 16 MiB`);
});
