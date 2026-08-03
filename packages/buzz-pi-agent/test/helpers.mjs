export const silentLogger = {
  debug() {},
  info() {},
  warn() {},
  error() {},
};

export function testConfig(overrides = {}) {
  return {
    contextLimitTokens: 150_000,
    compactionReserveTokens: 16_384,
    keepRecentTokens: 24_000,
    maxSessions: 4,
    sessionTtlMs: 60_000,
    sweepIntervalMs: 60_000,
    maxLineBytes: 10_000_000,
    maxActiveRequests: 64,
    maxOutputQueueMessages: 2_048,
    maxOutputQueueBytes: 16 * 1_024 * 1_024,
    stateDir: "/tmp/buzz-pi-agent-tests",
    maxPersistedConversations: 100,
    persistedConversationTtlMs: 90 * 24 * 60 * 60 * 1_000,
    maxPendingResetTombstones: 512,
    maxRetainedResetTombstones: 512,
    resetTombstoneTtlMs: 30 * 24 * 60 * 60 * 1_000,
    conversationLeaseMs: 60 * 60 * 1_000,
    maxSessionFileBytes: 64 * 1_024 * 1_024,
    maxPendingSessionEvents: 256,
    runtimeRequestTimeoutMs: 30 * 60 * 1_000,
    runtimeControlTimeoutMs: 2 * 60 * 1_000,
    runtimeInterruptTimeoutMs: 1_200,
    maxRuntimeIpcQueueMessages: 1_024,
    maxRuntimeIpcQueueBytes: 16 * 1_024 * 1_024,
    maxResourceFileBytes: 1 * 1_024 * 1_024,
    maxResourceTotalBytes: 16 * 1_024 * 1_024,
    maxResourceFiles: 512,
    maxResourceEntries: 4_096,
    maxResourceDepth: 16,
    trustProjectOverride: undefined,
    logLevel: "error",
    ...overrides,
  };
}

export function fakeHandle(overrides = {}) {
  let busy = false;
  let disposed = false;
  let model = overrides.model ?? "provider/model";
  let thinkingLevel = overrides.thinkingLevel ?? "off";
  const handle = {
    piSessionId: overrides.piSessionId ?? "pi_test",
    sessionFile: overrides.sessionFile ?? "/tmp/pi_test.jsonl",
    cwd: overrides.cwd ?? "/tmp",
    get isBusy() {
      return busy;
    },
    get isValid() {
      return !disposed;
    },
    async prompt(text, images = []) {
      busy = true;
      try {
        if (overrides.prompt) return await overrides.prompt(text, images);
        return "end_turn";
      } finally {
        busy = false;
      }
    },
    async steer(text) {
      await overrides.steer?.(text);
    },
    async abort() {
      busy = false;
      await overrides.abort?.();
    },
    async setModel(modelId) {
      await overrides.setModel?.(modelId);
      model = modelId;
    },
    async setThinkingLevel(level) {
      await overrides.setThinkingLevel?.(level);
      thinkingLevel = level;
    },
    async reload() {
      return handle.getResources();
    },
    async reset() {
      return {
        previousPiSessionId: handle.piSessionId,
        resources: handle.getResources(),
      };
    },
    getModels() {
      return overrides.models ?? [{ id: "provider/model", name: "Model" }];
    },
    getThinkingLevels() {
      return overrides.thinkingLevels ?? ["off", "medium", "high"];
    },
    getResources() {
      return {
        extensions: 2,
        skills: 3,
        prompts: 4,
        contextFiles: 1,
        errors: [],
        projectTrusted: false,
        commands: [],
      };
    },
    getContextSnapshot() {
      return {
        usedTokens: 12_345,
        limitTokens: 150_000,
        effectiveLimitTokens: 150_000,
        compactionThresholdTokens: 133_616,
        autoCompaction: true,
        compacting: false,
        model,
        thinkingLevel,
        piSessionId: handle.piSessionId,
      };
    },
    getUsageSnapshot() {
      return {
        contextTokens: 12_345,
        accumulatedInputTokens: 10_000,
        accumulatedOutputTokens: 2_000,
        accumulatedCachedInputTokens: 5_000,
        accumulatedCost: 0.12,
        model: "provider/model",
      };
    },
    async dispose() {
      disposed = true;
      await overrides.dispose?.();
    },
    get disposed() {
      return disposed;
    },
  };
  return handle;
}

export class MemoryWriter {
  messages = [];
  ended = false;

  write(value) {
    this.messages.push(structuredClone(value));
  }

  async end() {
    this.ended = true;
  }
}
