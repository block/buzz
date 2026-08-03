import { homedir } from "node:os";
import { join, resolve } from "node:path";

export interface AdapterConfig {
  contextLimitTokens: number;
  compactionReserveTokens: number;
  keepRecentTokens: number;
  maxSessions: number;
  sessionTtlMs: number;
  sweepIntervalMs: number;
  maxLineBytes: number;
  maxActiveRequests: number;
  maxOutputQueueMessages: number;
  maxOutputQueueBytes: number;
  stateDir: string;
  maxPersistedConversations: number;
  persistedConversationTtlMs: number;
  maxPendingResetTombstones: number;
  maxRetainedResetTombstones: number;
  resetTombstoneTtlMs: number;
  conversationLeaseMs: number;
  maxSessionFileBytes: number;
  maxPendingSessionEvents: number;
  runtimeRequestTimeoutMs: number;
  runtimeControlTimeoutMs: number;
  runtimeInterruptTimeoutMs: number;
  maxRuntimeIpcQueueMessages: number;
  maxRuntimeIpcQueueBytes: number;
  maxResourceFileBytes: number;
  maxResourceTotalBytes: number;
  maxResourceFiles: number;
  maxResourceEntries: number;
  maxResourceDepth: number;
  trustProjectOverride: boolean | undefined;
  logLevel: "debug" | "info" | "warn" | "error";
}

const DEFAULT_CONTEXT_LIMIT = 150_000;
const DEFAULT_RESERVE = 16_384;
const MAX_CONTEXT_LIMIT = 150_000;
const MAX_SESSIONS = 128;
const MAX_SESSION_TTL_MS = 24 * 60 * 60 * 1_000;
const MAX_SWEEP_INTERVAL_MS = 60 * 60 * 1_000;
const MAX_LINE_BYTES = 64 * 1_024 * 1_024;
const MAX_PERSISTED_TTL_MS = 365 * 24 * 60 * 60 * 1_000;
const MAX_CONVERSATION_LEASE_MS = 24 * 60 * 60 * 1_000;
const MIN_SESSION_FILE_BYTES = 1 * 1_024 * 1_024;
const MAX_SESSION_FILE_BYTES = 512 * 1_024 * 1_024;
const MAX_RUNTIME_REQUEST_TIMEOUT_MS = 6 * 60 * 60 * 1_000;
const MAX_RUNTIME_CONTROL_TIMEOUT_MS = 30 * 60 * 1_000;
const MAX_RUNTIME_INTERRUPT_TIMEOUT_MS = 60 * 1_000;
const MAX_RESOURCE_FILE_BYTES = 16 * 1_024 * 1_024;
const MAX_RESOURCE_TOTAL_BYTES = 64 * 1_024 * 1_024;
const MAX_RESOURCE_FILES = 4_096;
const MAX_RESOURCE_ENTRIES = 16_384;
const MAX_RESOURCE_DEPTH = 64;

export function loadConfig(
  env: NodeJS.ProcessEnv = process.env,
): AdapterConfig {
  const config: AdapterConfig = {
    contextLimitTokens: boundedPositiveInt(
      env.BUZZ_PI_CONTEXT_LIMIT,
      DEFAULT_CONTEXT_LIMIT,
      "BUZZ_PI_CONTEXT_LIMIT",
      MAX_CONTEXT_LIMIT,
    ),
    compactionReserveTokens: boundedPositiveInt(
      env.BUZZ_PI_COMPACTION_RESERVE,
      DEFAULT_RESERVE,
      "BUZZ_PI_COMPACTION_RESERVE",
      MAX_CONTEXT_LIMIT - 1,
    ),
    keepRecentTokens: boundedPositiveInt(
      env.BUZZ_PI_KEEP_RECENT_TOKENS,
      24_000,
      "BUZZ_PI_KEEP_RECENT_TOKENS",
      MAX_CONTEXT_LIMIT - 1,
    ),
    // Buzz's ACP pool normally disposes idle sessions first (8 sessions / 30m).
    // These are a defensive backstop for non-Buzz or interrupted callers.
    maxSessions: boundedPositiveInt(
      env.BUZZ_PI_MAX_SESSIONS,
      12,
      "BUZZ_PI_MAX_SESSIONS",
      MAX_SESSIONS,
    ),
    sessionTtlMs: boundedPositiveInt(
      env.BUZZ_PI_SESSION_TTL_MS,
      45 * 60 * 1_000,
      "BUZZ_PI_SESSION_TTL_MS",
      MAX_SESSION_TTL_MS,
    ),
    sweepIntervalMs: boundedPositiveInt(
      env.BUZZ_PI_SWEEP_INTERVAL_MS,
      5 * 60 * 1_000,
      "BUZZ_PI_SWEEP_INTERVAL_MS",
      MAX_SWEEP_INTERVAL_MS,
    ),
    maxLineBytes: boundedPositiveInt(
      env.BUZZ_PI_MAX_LINE_BYTES,
      10_000_000,
      "BUZZ_PI_MAX_LINE_BYTES",
      MAX_LINE_BYTES,
    ),
    maxActiveRequests: boundedPositiveInt(
      env.BUZZ_PI_MAX_ACTIVE_REQUESTS,
      64,
      "BUZZ_PI_MAX_ACTIVE_REQUESTS",
      512,
    ),
    maxOutputQueueMessages: boundedPositiveInt(
      env.BUZZ_PI_MAX_OUTPUT_QUEUE_MESSAGES,
      2_048,
      "BUZZ_PI_MAX_OUTPUT_QUEUE_MESSAGES",
      8_192,
    ),
    maxOutputQueueBytes: boundedPositiveInt(
      env.BUZZ_PI_MAX_OUTPUT_QUEUE_BYTES,
      16 * 1_024 * 1_024,
      "BUZZ_PI_MAX_OUTPUT_QUEUE_BYTES",
      64 * 1_024 * 1_024,
    ),
    stateDir: resolve(expandHome(env.BUZZ_PI_STATE_DIR ?? "~/.pi/agent/buzz")),
    maxPersistedConversations: boundedPositiveInt(
      env.BUZZ_PI_MAX_PERSISTED_CONVERSATIONS,
      512,
      "BUZZ_PI_MAX_PERSISTED_CONVERSATIONS",
      512,
    ),
    persistedConversationTtlMs: boundedPositiveInt(
      env.BUZZ_PI_PERSISTED_CONVERSATION_TTL_MS,
      90 * 24 * 60 * 60 * 1_000,
      "BUZZ_PI_PERSISTED_CONVERSATION_TTL_MS",
      MAX_PERSISTED_TTL_MS,
    ),
    maxPendingResetTombstones: boundedPositiveInt(
      env.BUZZ_PI_MAX_PENDING_RESET_TOMBSTONES,
      512,
      "BUZZ_PI_MAX_PENDING_RESET_TOMBSTONES",
      512,
    ),
    maxRetainedResetTombstones: boundedPositiveInt(
      env.BUZZ_PI_MAX_RETAINED_RESET_TOMBSTONES,
      512,
      "BUZZ_PI_MAX_RETAINED_RESET_TOMBSTONES",
      512,
    ),
    resetTombstoneTtlMs: boundedPositiveInt(
      env.BUZZ_PI_RESET_TOMBSTONE_TTL_MS,
      30 * 24 * 60 * 60 * 1_000,
      "BUZZ_PI_RESET_TOMBSTONE_TTL_MS",
      365 * 24 * 60 * 60 * 1_000,
    ),
    conversationLeaseMs: boundedPositiveInt(
      env.BUZZ_PI_CONVERSATION_LEASE_MS,
      60 * 60 * 1_000,
      "BUZZ_PI_CONVERSATION_LEASE_MS",
      MAX_CONVERSATION_LEASE_MS,
    ),
    maxSessionFileBytes: boundedPositiveInt(
      env.BUZZ_PI_MAX_SESSION_FILE_BYTES,
      64 * 1_024 * 1_024,
      "BUZZ_PI_MAX_SESSION_FILE_BYTES",
      MAX_SESSION_FILE_BYTES,
    ),
    maxPendingSessionEvents: boundedPositiveInt(
      env.BUZZ_PI_MAX_PENDING_SESSION_EVENTS,
      256,
      "BUZZ_PI_MAX_PENDING_SESSION_EVENTS",
      512,
    ),
    runtimeRequestTimeoutMs: boundedPositiveInt(
      env.BUZZ_PI_RUNTIME_REQUEST_TIMEOUT_MS,
      110 * 60 * 1_000,
      "BUZZ_PI_RUNTIME_REQUEST_TIMEOUT_MS",
      MAX_RUNTIME_REQUEST_TIMEOUT_MS,
    ),
    runtimeControlTimeoutMs: boundedPositiveInt(
      env.BUZZ_PI_RUNTIME_CONTROL_TIMEOUT_MS,
      2 * 60 * 1_000,
      "BUZZ_PI_RUNTIME_CONTROL_TIMEOUT_MS",
      MAX_RUNTIME_CONTROL_TIMEOUT_MS,
    ),
    runtimeInterruptTimeoutMs: boundedPositiveInt(
      env.BUZZ_PI_RUNTIME_INTERRUPT_TIMEOUT_MS,
      1_200,
      "BUZZ_PI_RUNTIME_INTERRUPT_TIMEOUT_MS",
      MAX_RUNTIME_INTERRUPT_TIMEOUT_MS,
    ),
    maxRuntimeIpcQueueMessages: boundedPositiveInt(
      env.BUZZ_PI_MAX_RUNTIME_IPC_QUEUE_MESSAGES,
      1_024,
      "BUZZ_PI_MAX_RUNTIME_IPC_QUEUE_MESSAGES",
      4_096,
    ),
    maxRuntimeIpcQueueBytes: boundedPositiveInt(
      env.BUZZ_PI_MAX_RUNTIME_IPC_QUEUE_BYTES,
      16 * 1_024 * 1_024,
      "BUZZ_PI_MAX_RUNTIME_IPC_QUEUE_BYTES",
      64 * 1_024 * 1_024,
    ),
    maxResourceFileBytes: boundedPositiveInt(
      env.BUZZ_PI_MAX_RESOURCE_FILE_BYTES,
      1 * 1_024 * 1_024,
      "BUZZ_PI_MAX_RESOURCE_FILE_BYTES",
      MAX_RESOURCE_FILE_BYTES,
    ),
    maxResourceTotalBytes: boundedPositiveInt(
      env.BUZZ_PI_MAX_RESOURCE_TOTAL_BYTES,
      16 * 1_024 * 1_024,
      "BUZZ_PI_MAX_RESOURCE_TOTAL_BYTES",
      MAX_RESOURCE_TOTAL_BYTES,
    ),
    maxResourceFiles: boundedPositiveInt(
      env.BUZZ_PI_MAX_RESOURCE_FILES,
      512,
      "BUZZ_PI_MAX_RESOURCE_FILES",
      MAX_RESOURCE_FILES,
    ),
    maxResourceEntries: boundedPositiveInt(
      env.BUZZ_PI_MAX_RESOURCE_ENTRIES,
      4_096,
      "BUZZ_PI_MAX_RESOURCE_ENTRIES",
      MAX_RESOURCE_ENTRIES,
    ),
    maxResourceDepth: boundedPositiveInt(
      env.BUZZ_PI_MAX_RESOURCE_DEPTH,
      16,
      "BUZZ_PI_MAX_RESOURCE_DEPTH",
      MAX_RESOURCE_DEPTH,
    ),
    trustProjectOverride: optionalBoolean(env.BUZZ_PI_TRUST_PROJECT),
    logLevel: parseLogLevel(env.BUZZ_PI_LOG_LEVEL),
  };
  validateConfig(config);
  return config;
}

function validateConfig(config: AdapterConfig): void {
  if (config.compactionReserveTokens >= config.contextLimitTokens) {
    throw new Error(
      "BUZZ_PI_COMPACTION_RESERVE must be smaller than BUZZ_PI_CONTEXT_LIMIT",
    );
  }
  const threshold = config.contextLimitTokens - config.compactionReserveTokens;
  if (config.keepRecentTokens >= threshold) {
    throw new Error(
      "BUZZ_PI_KEEP_RECENT_TOKENS must be smaller than the compaction threshold",
    );
  }
  if (config.sweepIntervalMs > config.sessionTtlMs) {
    throw new Error(
      "BUZZ_PI_SWEEP_INTERVAL_MS must not exceed BUZZ_PI_SESSION_TTL_MS",
    );
  }
  if (config.conversationLeaseMs < config.sweepIntervalMs * 2) {
    throw new Error(
      "BUZZ_PI_CONVERSATION_LEASE_MS must be at least twice BUZZ_PI_SWEEP_INTERVAL_MS",
    );
  }
  if (config.runtimeInterruptTimeoutMs >= config.runtimeControlTimeoutMs) {
    throw new Error(
      "BUZZ_PI_RUNTIME_INTERRUPT_TIMEOUT_MS must be smaller than BUZZ_PI_RUNTIME_CONTROL_TIMEOUT_MS",
    );
  }
  if (config.maxSessionFileBytes < MIN_SESSION_FILE_BYTES) {
    throw new Error(
      `BUZZ_PI_MAX_SESSION_FILE_BYTES must be at least ${MIN_SESSION_FILE_BYTES}`,
    );
  }
  if (config.maxResourceTotalBytes < config.maxResourceFileBytes) {
    throw new Error(
      "BUZZ_PI_MAX_RESOURCE_TOTAL_BYTES must be at least BUZZ_PI_MAX_RESOURCE_FILE_BYTES",
    );
  }
  if (config.maxResourceEntries < config.maxResourceFiles) {
    throw new Error(
      "BUZZ_PI_MAX_RESOURCE_ENTRIES must be at least BUZZ_PI_MAX_RESOURCE_FILES",
    );
  }
}

function positiveInt(raw: string | undefined, fallback: number): number {
  if (raw === undefined || raw.trim() === "") return fallback;
  const value = Number(raw);
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new Error(
      `Expected a positive integer, received ${JSON.stringify(raw)}`,
    );
  }
  return value;
}

function boundedPositiveInt(
  raw: string | undefined,
  fallback: number,
  name: string,
  maximum: number,
): number {
  const value = positiveInt(raw, fallback);
  if (value > maximum) {
    throw new Error(`${name} must not exceed ${maximum}`);
  }
  return value;
}

function optionalBoolean(raw: string | undefined): boolean | undefined {
  if (raw === undefined || raw.trim() === "") return undefined;
  if (["1", "true", "yes", "on"].includes(raw.toLowerCase())) return true;
  if (["0", "false", "no", "off"].includes(raw.toLowerCase())) return false;
  throw new Error(`Expected a boolean, received ${JSON.stringify(raw)}`);
}

function parseLogLevel(raw: string | undefined): AdapterConfig["logLevel"] {
  if (raw === undefined || raw === "") return "info";
  if (["debug", "info", "warn", "error"].includes(raw)) {
    return raw as AdapterConfig["logLevel"];
  }
  throw new Error(`Invalid BUZZ_PI_LOG_LEVEL ${JSON.stringify(raw)}`);
}

function expandHome(path: string): string {
  if (path === "~") return homedir();
  if (path.startsWith("~/")) return join(homedir(), path.slice(2));
  return path;
}

export function effectiveContextLimit(
  modelContextWindow: number,
  configuredLimit: number,
): number {
  if (modelContextWindow <= 0) return configuredLimit;
  return Math.max(1, Math.min(configuredLimit, modelContextWindow));
}

export function logicalModelContextWindow(
  modelContextWindow: number,
  configuredLimit: number,
): number {
  if (modelContextWindow <= 0) return configuredLimit;
  return Math.min(modelContextWindow, configuredLimit);
}

export function compactionThreshold(
  effectiveLimitTokens: number,
  reserveTokens: number,
): number {
  return Math.max(1, effectiveLimitTokens - reserveTokens);
}

export interface EffectiveCompactionSettings {
  reserveTokens: number;
  keepRecentTokens: number;
  thresholdTokens: number;
}

/**
 * Clamp configured compaction settings to the active model's logical window.
 * This keeps small-window custom providers valid without weakening the cap.
 */
export function effectiveCompactionSettings(
  effectiveLimitTokens: number,
  configuredReserveTokens: number,
  configuredKeepRecentTokens: number,
): EffectiveCompactionSettings {
  if (!Number.isSafeInteger(effectiveLimitTokens) || effectiveLimitTokens < 8) {
    throw new Error(
      "The selected Pi model context window is too small to compact safely",
    );
  }
  const maximumReserve = Math.max(1, Math.floor(effectiveLimitTokens / 4));
  const reserveTokens = Math.min(configuredReserveTokens, maximumReserve);
  const thresholdTokens = effectiveLimitTokens - reserveTokens;
  const maximumKeepRecent = Math.max(1, Math.floor(thresholdTokens * 0.75));
  const keepRecentTokens = Math.min(
    configuredKeepRecentTokens,
    maximumKeepRecent,
  );
  return { reserveTokens, keepRecentTokens, thresholdTokens };
}
