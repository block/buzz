import { createHash, randomUUID } from "node:crypto";
import { readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import {
  type AgentSessionRuntime,
  type CompactionEntry,
  CONFIG_DIR_NAME,
  ModelRuntime,
  ProjectTrustStore,
  SessionManager,
  SettingsManager,
  createAgentSessionFromServices,
  createAgentSessionRuntime,
  createAgentSessionServices,
  estimateTokens,
  getAgentDir,
  type AgentSessionEvent,
  type CreateAgentSessionRuntimeFactory,
} from "@earendil-works/pi-coding-agent";
import type { AdapterConfig } from "./config.js";
import {
  effectiveContextLimit,
  effectiveCompactionSettings,
  logicalModelContextWindow,
} from "./config.js";
import {
  assertPiResourceBudget,
  assertPiResourceSnapshotsEqual,
  type ResourceBudgetSnapshot,
} from "./resource-budget.js";
import type {
  AcpImageBlock,
  AgentSessionFactory,
  AgentSessionHandle,
  BuzzSessionEvent,
  CompactionReason,
  ContextSnapshot,
  CreateSessionOptions,
  Logger,
  ModelDescriptor,
  ResourceSnapshot,
  SessionUsageSnapshot,
} from "./types.js";
import {
  assertWorkspaceIdentity,
  captureWorkspaceIdentity,
  type WorkspaceIdentity,
} from "./workspace.js";

const THINKING_LEVELS = [
  "off",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
];
const MAX_SESSION_TITLE_LENGTH = 256;
const MAX_PROVIDER_IMAGES = 32;
const MAX_PROVIDER_IMAGE_BYTES = 8 * 1024 * 1024;
const MAX_PROVIDER_IMAGE_BYTES_TOTAL = 16 * 1024 * 1024;
const ESTIMATED_PROVIDER_IMAGE_TOKENS = 4_096;
const MAX_PROVIDER_PAYLOAD_NODES = 100_000;
const MAX_PROVIDER_PAYLOAD_ENTRIES = 100_000;
const MAX_PROVIDER_PAYLOAD_TEXT_BYTES = 4 * 1_024 * 1_024;
const MAX_PROVIDER_SERIALIZED_BYTES = 8 * 1_024 * 1_024;
const MAX_ADVERTISED_MODELS = 512;
const SESSION_FILE_CONTROL_RESERVE_BYTES = 64 * 1_024;
const quotaInstalled = new WeakSet<SessionManager>();
const MAX_TOOL_EVENT_PAYLOAD_BYTES = 64_000;
const MAX_TOOL_EVENT_PAYLOAD_NODES = 2_048;
const MAX_TOOL_EVENT_CONTAINER_ITEMS = 200;
const MAX_TOOL_EVENT_DEPTH = 12;
const MAX_TOOL_EVENT_TEXT_CHARACTERS = 128_000;
const COMPACTION_ATTEMPT_ENTRY = "buzz.compaction_attempt.v1";
const LIFECYCLE_WATERMARK_ENTRY = "buzz.lifecycle_watermark.v1";
const LIFECYCLE_PENDING_ENTRY = "buzz.lifecycle_pending.v1";
const LIFECYCLE_ACK_ENTRY = "buzz.lifecycle_ack.v1";
const MAX_CHILD_LIFECYCLE_RECORD_BYTES = 48 * 1_024;
const MAX_CHILD_LIFECYCLE_ATTEMPT_BYTES = 2 * 1_024;
const MAX_CHILD_LIFECYCLE_ACK_BYTES = 1 * 1_024;
const MAX_CHILD_LIFECYCLE_WATERMARK_BYTES = 1 * 1_024;
const MAX_CHILD_ROLLBACK_MARKER_BYTES = 2 * 1_024;
const MAX_CHILD_LIFECYCLE_MARKERS = 65_536;
const LOWERCASE_UUID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u;

type PiSettingsStorage = Parameters<typeof SettingsManager.fromStorage>[0];

const extensionProviderTrackers = new WeakMap<
  ModelRuntime,
  ExtensionProviderTracker
>();

/**
 * Pi's ModelRuntime intentionally merges extension provider registrations.
 * A resource reload therefore needs an adapter-owned generation boundary: an
 * extension that disappeared (including after project trust is revoked) must
 * not leave executable provider code behind in the session registry.
 */
class ExtensionProviderTracker {
  private readonly providerIds = new Set<string>();
  private replacingGeneration = false;
  private readonly originalRegisterProvider: ModelRuntime["registerProvider"];
  private readonly originalRegisterNativeProvider: ModelRuntime["registerNativeProvider"];
  private readonly originalUnregisterProvider: ModelRuntime["unregisterProvider"];

  constructor(private readonly modelRuntime: ModelRuntime) {
    this.originalRegisterProvider = modelRuntime.registerProvider;
    this.originalRegisterNativeProvider = modelRuntime.registerNativeProvider;
    this.originalUnregisterProvider = modelRuntime.unregisterProvider;

    modelRuntime.registerProvider = ((providerId, config) => {
      this.originalRegisterProvider.call(modelRuntime, providerId, config);
      this.providerIds.add(providerId);
    }) as ModelRuntime["registerProvider"];
    modelRuntime.registerNativeProvider = ((provider) => {
      this.originalRegisterNativeProvider.call(modelRuntime, provider);
      this.providerIds.add(provider.id);
    }) as ModelRuntime["registerNativeProvider"];
    modelRuntime.unregisterProvider = ((providerId) => {
      this.originalUnregisterProvider.call(modelRuntime, providerId);
      this.providerIds.delete(providerId);
    }) as ModelRuntime["unregisterProvider"];
  }

  async replaceGeneration<T>(operation: () => Promise<T>): Promise<T> {
    if (this.replacingGeneration) {
      throw new Error("Pi extension provider reload is already active");
    }

    const previousProviderIds = [...this.providerIds];
    this.replacingGeneration = true;
    try {
      // Clear the complete old extension layer before extensions register the
      // new one. This prevents merge semantics from retaining project-only
      // fields when a global provider with the same id remains after revocation.
      for (const providerId of previousProviderIds) {
        this.modelRuntime.unregisterProvider(providerId);
      }
      return await operation();
    } finally {
      this.replacingGeneration = false;
    }
  }
}

class PiResourceGenerationGuard {
  private globalSnapshot: ResourceBudgetSnapshot | undefined;
  private loadedSnapshot: ResourceBudgetSnapshot | undefined;

  constructor(
    private readonly workspaceIdentity: WorkspaceIdentity,
    private readonly agentDir: string,
    private readonly options: CreateSessionOptions,
    private readonly config: AdapterConfig,
    private readonly logger: Logger,
  ) {}

  armGlobalBootstrap(): void {
    this.globalSnapshot = this.scan(false);
    this.loadedSnapshot = undefined;
  }

  verifyGlobalBootstrap(): void {
    if (!this.globalSnapshot) {
      throw new Error("Pi resource generation was not armed before discovery");
    }
    assertPiResourceSnapshotsEqual(this.globalSnapshot, this.scan(false));
  }

  verifyModelInitialization(): void {
    if (!this.globalSnapshot) {
      throw new Error(
        "Pi resource generation was not armed before model setup",
      );
    }
    const after = this.scan(false);
    const authPath = join(this.agentDir, "auth.json");
    const authWasPresent = this.globalSnapshot.fingerprints.some(
      (fingerprint) => fingerprint.path === authPath,
    );
    const createdAuth = after.fingerprints.find(
      (fingerprint) => fingerprint.path === authPath,
    );
    if (!authWasPresent && createdAuth) {
      assertPiResourceSnapshotsEqual(this.globalSnapshot, {
        ...after,
        files: after.files - 1,
        bytes: after.bytes - createdAuth.size,
        fingerprints: after.fingerprints.filter(
          (fingerprint) => fingerprint !== createdAuth,
        ),
      });
    } else {
      assertPiResourceSnapshotsEqual(this.globalSnapshot, after);
    }
    // ModelRuntime initializes a missing empty auth store. It is bounded and
    // now becomes part of the immutable loader-generation baseline.
    this.globalSnapshot = after;
  }

  resolveProjectTrust(cwd: string): boolean {
    // Pi invokes this only after its untrusted bootstrap extension pass. Fence
    // that pass before allowing a trust transition to expose project files.
    this.verifyGlobalBootstrap();
    const trust = resolveProjectTrust(
      cwd,
      this.agentDir,
      this.config,
      this.logger,
    );
    this.loadedSnapshot = this.scan(trust);
    return trust;
  }

  prepareReloadTrust(cwd: string): boolean {
    // AgentSession.reload() does not reuse the SDK's initial
    // resourceLoaderReloadOptions callback. Resolve and install the trust state
    // explicitly, while retaining one immutable full-generation baseline.
    const globalBefore = this.scan(false);
    const trust = resolveProjectTrust(
      cwd,
      this.agentDir,
      this.config,
      this.logger,
    );
    assertPiResourceSnapshotsEqual(globalBefore, this.scan(false));
    this.globalSnapshot = undefined;
    this.loadedSnapshot = this.scan(trust);
    return trust;
  }

  verifyLoadedGeneration(projectTrusted: boolean): void {
    if (!this.loadedSnapshot) {
      throw new Error("Pi resource generation omitted its trust boundary");
    }
    const after = this.scan(projectTrusted);
    assertPiResourceSnapshotsEqual(this.loadedSnapshot, after);
    this.loadedSnapshot = after;
    this.globalSnapshot = undefined;
  }

  private scan(projectTrusted: boolean): ResourceBudgetSnapshot {
    assertWorkspaceIdentity(this.workspaceIdentity);
    const snapshot = assertPiResourceBudget({
      cwd: this.workspaceIdentity.canonicalPath,
      agentDir: this.agentDir,
      projectTrusted,
      config: this.config,
      ...(this.options.systemPrompt === undefined
        ? {}
        : { systemPromptSource: this.options.systemPrompt }),
    });
    assertWorkspaceIdentity(this.workspaceIdentity);
    return snapshot;
  }
}

/**
 * Read the user's normal Pi settings while discarding every attempted write.
 * Buzz model/thinking changes and extension writes therefore stay local to
 * this runtime and can never mutate ~/.pi/agent/settings.json.
 */
export class ReadOnlySettingsStorage implements PiSettingsStorage {
  constructor(
    private readonly cwd: string,
    private readonly agentDir: string,
  ) {}

  withLock(
    scope: "global" | "project",
    fn: (current: string | undefined) => string | undefined,
  ): void {
    const path =
      scope === "global"
        ? join(this.agentDir, "settings.json")
        : join(this.cwd, CONFIG_DIR_NAME, "settings.json");
    let current: string | undefined;
    try {
      current = readFileSync(path, "utf8");
    } catch (error) {
      if (!isFileNotFound(error)) throw error;
    }
    // The callback reads current state and may return a replacement. Ignoring
    // that return value is the write-discard boundary.
    void fn(current);
  }
}

type PiModelLike = ReturnType<ModelRuntime["getModels"]>[number];

interface UsageOffsets {
  input: number;
  output: number;
  cached: number;
  cost: number;
}

export class PiAgentSessionFactory implements AgentSessionFactory {
  constructor(
    private readonly config: AdapterConfig,
    private readonly logger: Logger,
  ) {}

  async create(options: CreateSessionOptions): Promise<AgentSessionHandle> {
    return PiAgentSessionHandle.create(options, this.config, this.logger);
  }
}

class PiAgentSessionHandle implements AgentSessionHandle {
  private unsubscribe: (() => void) | undefined;
  private busy = false;
  private disposed = false;
  private failure: Error | undefined;
  private fatalDisposal: Promise<void> | undefined;
  private abortRequested = false;
  private projectTrusted = false;
  private resourceErrors: string[] = [];
  private providerGuardedAgent: object | undefined;
  private contextLimitFailureEmitted = false;
  private pendingCompactionReason: CompactionReason | undefined;
  private pendingCompactionBefore: number | null = null;
  private pendingCompactionId: string | undefined;
  private pendingCompactionAttemptEntryId: string | undefined;
  private lifecycleIndexLoaded = false;
  private lifecycleIndexedLeafId: string | null | undefined;
  private lifecycleWatermarkEntryId: string | undefined;
  private readonly pendingLifecycleEvents = new Map<
    string,
    { sourceEntryId: string; event: BuzzSessionEvent }
  >();
  private readonly acknowledgedLifecycleEvents = new Set<string>();
  private offsets: UsageOffsets = { input: 0, output: 0, cached: 0, cost: 0 };
  private lastUsage: SessionUsageSnapshot = {
    contextTokens: null,
    accumulatedInputTokens: 0,
    accumulatedOutputTokens: 0,
    accumulatedCachedInputTokens: 0,
    accumulatedCost: null,
    model: null,
  };

  private constructor(
    private readonly runtime: AgentSessionRuntime,
    private readonly options: CreateSessionOptions,
    private readonly config: AdapterConfig,
    private readonly logger: Logger,
    private readonly resourceGuard: PiResourceGenerationGuard,
  ) {}

  static async create(
    options: CreateSessionOptions,
    config: AdapterConfig,
    logger: Logger,
  ): Promise<PiAgentSessionHandle> {
    const workspaceIdentity = captureWorkspaceIdentity(
      options.requestedCwd ?? options.cwd,
      options.requestedCwd === undefined ? undefined : options.cwd,
    );
    const normalizedOptions: CreateSessionOptions = {
      ...options,
      cwd: workspaceIdentity.canonicalPath,
      requestedCwd: workspaceIdentity.requestedPath,
    };
    const agentDir = getAgentDir();
    const resourceGuard = new PiResourceGenerationGuard(
      workspaceIdentity,
      agentDir,
      normalizedOptions,
      config,
      logger,
    );
    resourceGuard.armGlobalBootstrap();
    const runtimeFactory: CreateAgentSessionRuntimeFactory = async ({
      cwd,
      sessionManager,
      sessionStartEvent,
    }) => {
      assertWorkspaceIdentity(workspaceIdentity);
      installSessionFileQuota(sessionManager, config.maxSessionFileBytes);
      const modelRuntime = await ModelRuntime.create({
        authPath: join(agentDir, "auth.json"),
        modelsPath: join(agentDir, "models.json"),
      });
      const providerTracker = new ExtensionProviderTracker(modelRuntime);
      extensionProviderTrackers.set(modelRuntime, providerTracker);
      const settingsManager = SettingsManager.fromStorage(
        new ReadOnlySettingsStorage(cwd, agentDir),
        {
          projectTrusted: false,
        },
      );
      await settingsManager.reload();
      resourceGuard.verifyModelInitialization();
      settingsManager.applyOverrides({
        compaction: {
          enabled: true,
          reserveTokens: config.compactionReserveTokens,
          keepRecentTokens: config.keepRecentTokens,
        },
      });
      const services = await providerTracker.replaceGeneration(() =>
        createAgentSessionServices({
          cwd,
          agentDir,
          modelRuntime,
          settingsManager,
          ...(normalizedOptions.systemPrompt === undefined
            ? {}
            : {
                resourceLoaderOptions: {
                  systemPrompt: normalizedOptions.systemPrompt,
                },
              }),
          resourceLoaderReloadOptions: {
            resolveProjectTrust: async () =>
              resourceGuard.resolveProjectTrust(cwd),
          },
        }),
      );
      resourceGuard.verifyLoadedGeneration(settingsManager.isProjectTrusted());
      const result = await createAgentSessionFromServices({
        services,
        sessionManager,
        ...(sessionStartEvent === undefined ? {} : { sessionStartEvent }),
      });
      resourceGuard.verifyLoadedGeneration(settingsManager.isProjectTrusted());
      return {
        ...result,
        services,
        diagnostics: services.diagnostics,
      };
    };

    if (normalizedOptions.persistedSessionFile) {
      assertSessionFileSizeWithinQuota(
        normalizedOptions.persistedSessionFile,
        config.maxSessionFileBytes,
      );
    }
    const sessionManager = normalizedOptions.persistedSessionFile
      ? SessionManager.open(
          normalizedOptions.persistedSessionFile,
          undefined,
          normalizedOptions.cwd,
        )
      : SessionManager.create(normalizedOptions.cwd);
    // SessionManager.open may migrate an older transcript by rewriting it.
    // Recheck immediately, before extensions or a provider can observe it.
    if (normalizedOptions.persistedSessionFile) {
      assertSessionFileSizeWithinQuota(
        normalizedOptions.persistedSessionFile,
        config.maxSessionFileBytes,
      );
    }
    const runtime = await createAgentSessionRuntime(runtimeFactory, {
      cwd: normalizedOptions.cwd,
      agentDir,
      sessionManager,
    });
    if (!normalizedOptions.persistedSessionFile) {
      applyFreshSessionTitle(runtime.session, normalizedOptions.title);
      persistFreshSession(runtime.session.sessionManager);
      const freshFile = runtime.session.sessionFile;
      if (freshFile)
        assertSessionFileSizeWithinQuota(freshFile, config.maxSessionFileBytes);
    }
    const handle = new PiAgentSessionHandle(
      runtime,
      normalizedOptions,
      config,
      logger,
      resourceGuard,
    );
    try {
      await handle.bindCurrentSession();
      return handle;
    } catch (error) {
      await runtime.dispose().catch(() => {});
      throw error;
    }
  }

  get piSessionId(): string {
    return this.runtime.session.sessionId;
  }

  get sessionFile(): string | undefined {
    return this.runtime.session.sessionFile;
  }

  get cwd(): string {
    return this.runtime.session.sessionManager.getCwd();
  }

  get isBusy(): boolean {
    return (
      this.busy ||
      this.runtime.session.isStreaming ||
      this.runtime.session.isCompacting
    );
  }

  get isValid(): boolean {
    return !this.disposed && this.failure === undefined;
  }

  async prompt(
    text: string,
    images: AcpImageBlock[] = [],
  ): Promise<"end_turn" | "cancelled" | "max_tokens"> {
    this.assertUsable();
    if (this.isBusy) throw new Error("prompt already in flight");

    const command = firstLine(text);
    if (command === "/context") {
      this.emitContextStatus();
      return "end_turn";
    }
    if (command === "/reload") {
      const resources = await this.reload();
      this.emitResourcesReloaded(resources);
      return "end_turn";
    }
    if (command === "/new") {
      throw new Error(
        "/new must be authorized and handled by Buzz, not by the Pi adapter",
      );
    }
    if (command === "/compact") {
      if (this.runtime.session.agent.state.messages.length === 0) {
        this.emitContextStatus(
          "Nothing to compact yet; this Pi session has no conversation history.",
        );
        return "end_turn";
      }
      try {
        await this.runManualCompaction("manual");
      } catch (error) {
        // The typed lifecycle failure is the user-facing result for an
        // explicit command. Returning a completed command prevents Buzz from
        // retrying a deterministic extension cancellation/provider failure.
        this.logger.warn("manual Pi compaction did not complete", {
          sessionId: this.options.acpSessionId,
          error: publicError(errorMessage(error)),
        });
      }
      return "end_turn";
    }

    this.contextLimitFailureEmitted = false;
    await this.compactBeforePromptIfNeeded(text, images.length);
    this.busy = true;
    this.abortRequested = false;
    try {
      const previousLastMessage = this.runtime.session.state.messages.at(-1);
      const previousLeafId = this.runtime.session.sessionManager.getLeafId();
      let rollbackAttempted = false;
      const rollbackTurn = (): void => {
        if (rollbackAttempted) return;
        rollbackAttempted = true;
        const currentLeafId = this.runtime.session.sessionManager.getLeafId();
        if (currentLeafId === previousLeafId) {
          // Preflight/quota guards can reject before Pi persists any part of
          // the turn. Rebuild runtime state, but do not consume the protected
          // rollback reserve with a redundant marker on every caller retry.
          this.runtime.session.agent.state.messages =
            this.runtime.session.sessionManager.buildSessionContext().messages;
          return;
        }
        // Pi's log remains append-only for diagnostics, but the active branch
        // must exclude cancelled or failed work before dispose(false)/resume.
        // branch() alone is an in-memory pointer, so append a context-free
        // marker on the restored branch to make that leaf durable in JSONL.
        if (previousLeafId === null)
          this.runtime.session.sessionManager.resetLeaf();
        else this.runtime.session.sessionManager.branch(previousLeafId);
        this.runtime.session.sessionManager.appendCustomEntry(
          "buzz.turn_rollback",
          { version: 1 },
        );
        this.runtime.session.agent.state.messages =
          this.runtime.session.sessionManager.buildSessionContext().messages;
      };
      try {
        await this.runtime.session.prompt(text, {
          source: "rpc",
          ...(images.length === 0
            ? {}
            : {
                images: images.map((image) => ({
                  type: "image" as const,
                  data: image.data,
                  mimeType: image.mimeType,
                })),
              }),
        });
        const last = this.runtime.session.state.messages.at(-1);
        const assistant =
          last !== previousLastMessage && last?.role === "assistant"
            ? last
            : undefined;
        if (this.abortRequested || assistant?.stopReason === "aborted") {
          rollbackTurn();
          return "cancelled";
        }
        if (assistant?.stopReason === "length") return "max_tokens";
        if (assistant?.stopReason === "error") {
          throw new Error(assistant.errorMessage || "Pi model request failed");
        }
        return "end_turn";
      } catch (error) {
        rollbackTurn();
        throw error;
      }
    } finally {
      this.busy = false;
      this.abortRequested = false;
    }
  }

  async steer(text: string): Promise<void> {
    this.assertUsable();
    if (!this.isBusy) {
      void this.prompt(text).catch((error: unknown) => {
        this.logger.error("detached steer turn failed", {
          sessionId: this.options.acpSessionId,
          error: errorMessage(error),
        });
      });
      return;
    }
    await this.runtime.session.steer(text);
  }

  async abort(): Promise<void> {
    if (this.disposed) return;
    this.abortRequested = true;
    await this.runtime.session.abort();
  }

  async setModel(modelId: string): Promise<void> {
    this.assertUsable();
    if (this.isBusy)
      throw new Error("cannot switch model while a turn is active");
    const model = resolveModel(this.runtime.services.modelRuntime, modelId);
    if (!model) throw new Error(`Unknown or unavailable Pi model ${modelId}`);
    await this.runtime.session.setModel(limitModelContext(model, this.config));
    this.applyCompactionSettings();
  }

  async setThinkingLevel(level: string): Promise<void> {
    this.assertUsable();
    if (!THINKING_LEVELS.includes(level))
      throw new Error(`Unsupported thinking level ${level}`);
    this.runtime.session.setThinkingLevel(level as never);
  }

  async reload(): Promise<ResourceSnapshot> {
    this.assertUsable();
    if (this.isBusy)
      throw new Error("cannot reload resources while a turn is active");
    return this.reloadResources();
  }

  private async reloadResources(): Promise<ResourceSnapshot> {
    this.assertUsable();
    try {
      const trust = this.resourceGuard.prepareReloadTrust(this.cwd);
      this.runtime.services.settingsManager.setProjectTrusted(trust);
      const providerTracker = extensionProviderTrackers.get(
        this.runtime.services.modelRuntime,
      );
      if (!providerTracker) {
        throw new Error(
          "Installed Pi SDK is incompatible with Buzz extension provider reload",
        );
      }
      await providerTracker.replaceGeneration(() =>
        this.runtime.session.reload(),
      );
      this.resourceGuard.verifyLoadedGeneration(
        this.runtime.services.settingsManager.isProjectTrusted(),
      );
      this.projectTrusted =
        this.runtime.services.settingsManager.isProjectTrusted();
      this.reconcileCurrentModelWithRegistry();
      await this.limitCurrentModel();
      this.applyCompactionSettings();
      this.installFinalProviderGuard();
      return this.getResourceSnapshot();
    } catch (error) {
      throw this.poisonAfterReloadFailure(error);
    }
  }

  async reset(): Promise<{
    previousPiSessionId: string;
    resources: ResourceSnapshot;
  }> {
    this.assertUsable();
    if (this.isBusy) await this.abort();
    this.updateUsageSnapshot();
    this.offsets = {
      input: this.lastUsage.accumulatedInputTokens,
      output: this.lastUsage.accumulatedOutputTokens,
      cached: this.lastUsage.accumulatedCachedInputTokens,
      cost: this.lastUsage.accumulatedCost ?? this.offsets.cost,
    };
    const previousPiSessionId = this.piSessionId;
    this.unsubscribe?.();
    await this.runtime.newSession();
    this.resetLifecycleIndex();
    applyFreshSessionTitle(this.runtime.session, this.options.title);
    persistFreshSession(this.runtime.session.sessionManager);
    await this.bindCurrentSession();
    return { previousPiSessionId, resources: this.getResourceSnapshot() };
  }

  getModels(): ModelDescriptor[] {
    return this.runtime.services.modelRuntime
      .getAvailableSnapshot()
      .filter((model) => `${model.provider}/${model.id}`.length <= 256)
      .slice(0, MAX_ADVERTISED_MODELS)
      .map(describeModel);
  }

  getThinkingLevels(): string[] {
    return [...THINKING_LEVELS];
  }

  getResources(): ResourceSnapshot {
    return this.getResourceSnapshot();
  }

  getContextSnapshot(): ContextSnapshot {
    const effectiveLimitTokens = this.getEffectiveLimit();
    return {
      usedTokens: this.currentEstimatedContextTokens(),
      limitTokens: this.config.contextLimitTokens,
      effectiveLimitTokens,
      compactionThresholdTokens: this.getCompactionThreshold(),
      autoCompaction: this.runtime.session.autoCompactionEnabled,
      compacting: this.runtime.session.isCompacting,
      model: this.modelId(),
      thinkingLevel: this.runtime.session.thinkingLevel,
      piSessionId: this.piSessionId,
    };
  }

  getUsageSnapshot(): SessionUsageSnapshot {
    this.updateUsageSnapshot();
    return { ...this.lastUsage };
  }

  async replayLifecycleEvents(): Promise<void> {
    this.assertUsable();
    this.ensureLifecycleIndexLoaded();
    this.reconcileMissingCompactionEvents();
    for (const [deliveryId, pending] of this.pendingLifecycleEvents) {
      if (this.acknowledgedLifecycleEvents.has(deliveryId)) continue;
      await this.options.eventSink.buzzSessionEvent(
        this.options.acpSessionId,
        pending.event,
        deliveryId,
      );
    }
  }

  async acknowledgeLifecycleEvent(deliveryId: string): Promise<void> {
    this.assertUsable();
    if (!LOWERCASE_UUID_PATTERN.test(deliveryId)) {
      throw new Error("lifecycle deliveryId must be a lowercase UUID");
    }
    this.ensureLifecycleIndexLoaded();
    if (this.acknowledgedLifecycleEvents.has(deliveryId)) return;
    if (!this.pendingLifecycleEvents.has(deliveryId)) {
      throw new Error("cannot acknowledge an unknown Pi lifecycle delivery");
    }
    this.runtime.session.sessionManager.appendCustomEntry(LIFECYCLE_ACK_ENTRY, {
      version: 1,
      deliveryId,
    });
    this.lifecycleIndexedLeafId =
      this.runtime.session.sessionManager.getLeafId();
    this.acknowledgedLifecycleEvents.add(deliveryId);
    this.pendingLifecycleEvents.delete(deliveryId);
  }

  async dispose(): Promise<void> {
    if (this.disposed) {
      await this.fatalDisposal;
      return;
    }
    this.disposed = true;
    this.unsubscribe?.();
    this.unsubscribe = undefined;
    this.fatalDisposal ??= this.disposeRuntime();
    await this.fatalDisposal;
  }

  private async bindCurrentSession(): Promise<void> {
    // Establish a durable feature boundary before this Buzz-managed generation
    // can compact. Legacy Pi compactions before this marker are intentionally
    // not backfilled as crash-gap notices.
    this.ensureLifecycleIndexLoaded();
    this.projectTrusted =
      this.runtime.services.settingsManager.isProjectTrusted();
    await this.limitCurrentModel();
    this.applyCompactionSettings();
    await this.runtime.session.bindExtensions({
      mode: "print",
      commandContextActions: {
        waitForIdle: () => this.runtime.session.waitForIdle(),
        newSession: async () => {
          return { cancelled: true };
        },
        fork: async (_entryId, _options) => {
          return { cancelled: true };
        },
        navigateTree: async (entryId, options) => {
          const result = await this.runtime.session.navigateTree(
            entryId,
            options,
          );
          return { cancelled: result.cancelled };
        },
        switchSession: async (_path, _options) => {
          return { cancelled: true };
        },
        reload: async () => {
          const resources = await this.reloadResources();
          this.emitResourcesReloaded(resources);
        },
      },
      shutdownHandler: () => {
        this.logger.warn(
          "Pi extension requested shutdown; ignored in Buzz headless mode",
          {
            sessionId: this.options.acpSessionId,
          },
        );
      },
      onError: (error) => {
        this.resourceErrors.push(publicError(error.error));
        this.resourceErrors = this.resourceErrors.slice(-50);
        this.logger.warn("Pi extension error", {
          sessionId: this.options.acpSessionId,
          extension: error.extensionPath,
          event: error.event,
          error: error.error,
        });
      },
    });
    // Extensions can register providers or select a model during session_start.
    // Re-apply the logical model window, then guard the actual LLM context and
    // provider payload at their final dispatch boundaries.
    await this.limitCurrentModel();
    this.installFinalProviderGuard();
    this.unsubscribe?.();
    this.unsubscribe = this.runtime.session.subscribe((event) =>
      this.handleEvent(event),
    );
  }

  private handleEvent(event: AgentSessionEvent): void {
    if (event.type === "message_update") {
      const update = event.assistantMessageEvent;
      if (update.type === "text_delta") {
        this.options.eventSink.sessionUpdate(this.options.acpSessionId, {
          sessionUpdate: "agent_message_chunk",
          content: { type: "text", text: update.delta },
        });
      } else if (update.type === "thinking_delta") {
        this.options.eventSink.sessionUpdate(this.options.acpSessionId, {
          sessionUpdate: "agent_thought_chunk",
          content: { type: "text", text: update.delta },
        });
      }
      return;
    }
    if (event.type === "tool_execution_start") {
      this.options.eventSink.sessionUpdate(this.options.acpSessionId, {
        sessionUpdate: "tool_call",
        toolCallId: event.toolCallId,
        title: event.toolName,
        kind: toolKind(event.toolName),
        status: "in_progress",
        rawInput: truncatePayload(event.args),
      });
      return;
    }
    if (event.type === "tool_execution_update") {
      this.options.eventSink.sessionUpdate(this.options.acpSessionId, {
        sessionUpdate: "tool_call_update",
        toolCallId: event.toolCallId,
        status: "in_progress",
        rawOutput: truncatePayload(event.partialResult),
      });
      return;
    }
    if (event.type === "tool_execution_end") {
      this.options.eventSink.sessionUpdate(this.options.acpSessionId, {
        sessionUpdate: "tool_call_update",
        toolCallId: event.toolCallId,
        status: event.isError ? "failed" : "completed",
        rawOutput: truncatePayload(event.result),
      });
      return;
    }
    if (event.type === "compaction_start") {
      if (!this.pendingCompactionReason) {
        this.pendingCompactionReason = event.reason;
        this.pendingCompactionBefore = this.currentEstimatedContextTokens();
      }
      this.pendingCompactionId ??= randomUUID();
      if (this.pendingCompactionAttemptEntryId === undefined) {
        this.pendingCompactionAttemptEntryId =
          this.runtime.session.sessionManager.appendCustomEntry(
            COMPACTION_ATTEMPT_ENTRY,
            {
              version: 1,
              compactionId: this.pendingCompactionId,
              reason: this.pendingCompactionReason,
              beforeTokens: this.pendingCompactionBefore,
              startedAt: new Date().toISOString(),
            },
          );
      }
      return;
    }
    if (event.type === "compaction_end") {
      this.emitCompactionResult(event);
      return;
    }
    if (event.type === "agent_settled") {
      const usage = this.getUsageSnapshot();
      this.options.eventSink.usageUpdate(
        this.options.acpSessionId,
        usage,
        this.getEffectiveLimit(),
      );
    }
  }

  private async compactBeforePromptIfNeeded(
    text: string,
    imageCount: number,
  ): Promise<void> {
    const context = this.runtime.session.getContextUsage();
    const state = this.runtime.session.agent.state;
    const baseEstimate = estimateProviderContextTokens({
      systemPrompt: state.systemPrompt,
      messages: state.messages,
      tools: state.tools,
    });
    const currentTokens = Math.max(context?.tokens ?? 0, baseEstimate);
    const estimatedIncoming =
      estimateAdaptiveTextTokens(text) + imageCount * 4_096;
    const fixedContextTokens = estimateProviderContextTokens({
      systemPrompt: state.systemPrompt,
      messages: [],
      tools: state.tools,
    });
    if (fixedContextTokens + estimatedIncoming > this.getEffectiveLimit()) {
      const error = new Error(
        `BUZZ_CONTEXT_LIMIT: the system prompt, tools, and incoming prompt exceed the ${this.getEffectiveLimit()}-token effective limit even without conversation history`,
      );
      this.emitContextLimitFailure(error);
      throw error;
    }
    if (currentTokens + estimatedIncoming < this.getCompactionThreshold())
      return;
    // A fresh session has no transcript that compaction can remove. A large
    // first request may legitimately sit between the proactive threshold and
    // the effective ceiling; let the final provider guard decide it.
    if (state.messages.length === 0) return;
    await this.runManualCompaction("preflight");
  }

  private async runManualCompaction(
    reason: "manual" | "preflight",
  ): Promise<void> {
    this.pendingCompactionReason = reason;
    this.pendingCompactionBefore = this.currentEstimatedContextTokens();
    this.pendingCompactionId = randomUUID();
    try {
      await this.runtime.session.compact(
        "Preserve decisions, unresolved tasks, constraints, identifiers, paths, commands, and the information needed to continue accurately.",
      );
    } catch (error) {
      if (this.pendingCompactionReason) {
        this.emitCompactionFailure(reason, errorMessage(error), false);
        this.clearPendingCompaction();
      }
      if (reason === "preflight") {
        if (isContextLimitError(error)) throw error;
        throw new Error(
          `BUZZ_CONTEXT_LIMIT: preflight compaction could not create safe room for this turn: ${publicError(errorMessage(error))}`,
        );
      }
      throw error;
    }
  }

  private emitCompactionResult(
    event: Extract<AgentSessionEvent, { type: "compaction_end" }>,
  ): void {
    const reason = this.pendingCompactionReason ?? event.reason;
    const compactionId = this.pendingCompactionId ?? randomUUID();
    const beforeTokens =
      event.result?.tokensBefore ?? this.pendingCompactionBefore;
    if (event.result && !event.aborted && !event.errorMessage) {
      const afterTokens = event.result.estimatedTokensAfter ?? null;
      const lifecycleEvent: BuzzSessionEvent = {
        type: "compaction_completed",
        compactionId,
        timestamp: new Date().toISOString(),
        message: contextMessage("compacted", beforeTokens, afterTokens),
        piSessionId: this.piSessionId,
        reason,
        beforeTokens,
        afterTokens,
        limitTokens: this.config.contextLimitTokens,
        effectiveLimitTokens: this.getEffectiveLimit(),
        compactionThresholdTokens: this.getCompactionThreshold(),
        willRetry: event.willRetry,
        fromExtension: this.lastCompactionFromExtension(),
      };
      const compactionEntry = this.latestPersistedCompaction();
      if (compactionEntry) {
        const deliveryId = this.persistCompactionLifecycleEvent(
          compactionEntry,
          lifecycleEvent,
        );
        void Promise.resolve(
          this.options.eventSink.buzzSessionEvent(
            this.options.acpSessionId,
            lifecycleEvent,
            deliveryId,
          ),
        ).catch((error: unknown) => {
          // The child-side pending entry remains authoritative. A poisoned IPC
          // host will exit and replay it after the outer session is recreated.
          this.logger.error("failed to deliver durable Pi lifecycle event", {
            sessionId: this.options.acpSessionId,
            error: errorMessage(error),
          });
        });
      } else {
        // Synthetic tests/extensions can emit compaction_end without Pi having
        // appended a compaction entry. There is no durable side effect to
        // reconcile in that compatibility path.
        this.options.eventSink.buzzSessionEvent(
          this.options.acpSessionId,
          lifecycleEvent,
        );
      }
    } else {
      this.emitCompactionFailure(
        reason,
        event.errorMessage ??
          (event.aborted ? "Compaction was cancelled" : "Compaction failed"),
        event.aborted,
        event.willRetry,
        false,
      );
    }
    this.clearPendingCompaction();
  }

  private emitCompactionFailure(
    reason: CompactionReason,
    error: string,
    aborted: boolean,
    willRetry = false,
    fromExtension = false,
  ): void {
    const compactionId = this.pendingCompactionId ?? randomUUID();
    const safeError = publicError(error);
    this.options.eventSink.buzzSessionEvent(this.options.acpSessionId, {
      type: "compaction_failed",
      compactionId,
      timestamp: new Date().toISOString(),
      message: boundedString(
        `Pi could not compact this thread's context: ${safeError}`,
        1_024,
      ),
      piSessionId: this.piSessionId,
      reason,
      beforeTokens: this.pendingCompactionBefore,
      limitTokens: this.config.contextLimitTokens,
      effectiveLimitTokens: this.getEffectiveLimit(),
      compactionThresholdTokens: this.getCompactionThreshold(),
      error: boundedString(safeError, 1_024),
      aborted,
      willRetry,
      fromExtension,
    });
  }

  private emitContextStatus(messageOverride?: string): void {
    const snapshot = this.getContextSnapshot();
    const remainingTokens =
      snapshot.usedTokens === null
        ? null
        : Math.max(0, snapshot.effectiveLimitTokens - snapshot.usedTokens);
    const percent =
      snapshot.usedTokens === null
        ? null
        : (snapshot.usedTokens / snapshot.effectiveLimitTokens) * 100;
    const message =
      messageOverride ??
      (snapshot.usedTokens === null
        ? `Pi context usage is being recalculated after compaction. Auto-compaction starts near ${formatTokens(snapshot.compactionThresholdTokens)} (logical cap ${formatTokens(snapshot.effectiveLimitTokens)}).`
        : `Estimated Pi provider context: ${formatTokens(snapshot.usedTokens)} / ${formatTokens(snapshot.effectiveLimitTokens)} (${percent?.toFixed(1)}%). Auto-compaction starts near ${formatTokens(snapshot.compactionThresholdTokens)}.`);
    this.options.eventSink.buzzSessionEvent(this.options.acpSessionId, {
      type: "context_status",
      timestamp: new Date().toISOString(),
      message,
      piSessionId: snapshot.piSessionId,
      usedTokens: snapshot.usedTokens,
      remainingTokens,
      percent,
      limitTokens: snapshot.limitTokens,
      effectiveLimitTokens: snapshot.effectiveLimitTokens,
      compactionThresholdTokens: snapshot.compactionThresholdTokens,
      autoCompaction: snapshot.autoCompaction,
      compacting: snapshot.compacting,
      model: snapshot.model,
    });
  }

  private emitResourcesReloaded(resources: ResourceSnapshot): void {
    const message = `Reloaded Pi resources: ${resources.extensions} extensions, ${resources.skills} skills, ${resources.prompts} prompts, and ${resources.contextFiles} context files.`;
    this.options.eventSink.buzzSessionEvent(this.options.acpSessionId, {
      type: "extensions_reloaded",
      timestamp: new Date().toISOString(),
      message,
      piSessionId: this.piSessionId,
      extensions: resources.extensions,
      skills: resources.skills,
      prompts: resources.prompts,
      contextFiles: resources.contextFiles,
      errors: resources.errors,
      projectTrusted: resources.projectTrusted,
    });
    this.emitAvailableCommands(resources.commands);
  }

  private getResourceSnapshot(): ResourceSnapshot {
    const loader = this.runtime.session.resourceLoader;
    const extensions = loader.getExtensions();
    const skills = loader.getSkills();
    const prompts = loader.getPrompts();
    const reserved = new Set(["new", "context", "reload", "compact"]);
    const commands = [
      ...this.runtime.session.extensionRunner
        .getRegisteredCommands()
        .map((command) => ({
          name: command.invocationName,
          description: command.description,
        })),
      ...prompts.prompts.map((prompt) => ({
        name: prompt.name,
        description: prompt.description,
      })),
      ...(this.runtime.services.settingsManager.getEnableSkillCommands()
        ? skills.skills.map((skill) => ({
            name: `skill:${skill.name}`,
            description: skill.description,
          }))
        : []),
    ]
      .filter(
        (command) =>
          !reserved.has(command.name) &&
          command.name.length > 0 &&
          command.name.length <= 128 &&
          /^[a-zA-Z0-9:_-]+$/.test(command.name),
      )
      .map((command) => ({
        name: command.name,
        description: boundedString(command.description || "Pi command", 256),
      }));
    const uniqueCommands = dedupeCommands(commands).slice(0, 100);
    const runtimeDiagnostics = [
      ...this.runtime.diagnostics,
      ...this.runtime.services.diagnostics,
    ].map(
      (diagnostic) =>
        `Pi runtime ${diagnostic.type}: ${publicError(diagnostic.message)}`,
    );
    const errors = [
      ...runtimeDiagnostics,
      ...this.resourceErrors,
      ...extensions.errors.map(
        (error) => `Extension: ${publicError(error.error)}`,
      ),
      ...skills.diagnostics.map(
        (diagnostic) => `Skill: ${publicError(diagnostic.message)}`,
      ),
      ...prompts.diagnostics.map(
        (diagnostic) => `Prompt: ${publicError(diagnostic.message)}`,
      ),
    ];
    return {
      extensions: extensions.extensions.length,
      skills: skills.skills.length,
      prompts: prompts.prompts.length,
      contextFiles: loader.getAgentsFiles().agentsFiles.length,
      errors: [...new Set(errors)]
        .slice(0, 20)
        .map((error) => boundedString(error, 512)),
      projectTrusted: this.projectTrusted,
      commands: uniqueCommands,
    };
  }

  private emitAvailableCommands(commands: ResourceSnapshot["commands"]): void {
    this.options.eventSink.sessionUpdate(this.options.acpSessionId, {
      sessionUpdate: "available_commands_update",
      availableCommands: [
        {
          name: "context",
          description: "Show live context usage and compaction limit",
        },
        {
          name: "reload",
          description: "Hot-reload Pi extensions, skills, prompts, and context",
        },
        {
          name: "compact",
          description: "Compact this thread's Pi context now",
        },
        ...commands,
      ],
    });
  }

  private async limitCurrentModel(): Promise<void> {
    const model = this.runtime.session.model;
    if (!model) return;
    const limited = limitModelContext(model, this.config);
    if (limited.contextWindow !== model.contextWindow) {
      // This is an adapter-local logical ceiling, not a user model choice.
      // Mutate only the active runtime state so Pi's settings.json is untouched.
      this.runtime.session.agent.state.model = limited;
    }
  }

  private reconcileCurrentModelWithRegistry(): void {
    const current = this.runtime.session.model;
    if (!current) return;
    const registered = this.runtime.services.modelRuntime.getModel(
      current.provider,
      current.id,
    );
    // Pi deliberately retains the old active model when a provider disappears.
    // Buzz must instead fail closed: a model backed only by a revoked extension
    // cannot service one more request after /reload.
    // AgentSession.model is documented as optional, although the lower-level
    // pi-agent-core state type currently declares it as required.
    (
      this.runtime.session.agent.state as unknown as {
        model: ReturnType<ModelRuntime["getModel"]>;
      }
    ).model = registered;
  }

  private applyCompactionSettings(): void {
    const settings = effectiveCompactionSettings(
      this.getEffectiveLimit(),
      this.config.compactionReserveTokens,
      this.config.keepRecentTokens,
    );
    this.runtime.services.settingsManager.applyOverrides({
      compaction: {
        enabled: true,
        reserveTokens: settings.reserveTokens,
        keepRecentTokens: settings.keepRecentTokens,
      },
    });
  }

  private getEffectiveLimit(): number {
    return effectiveContextLimit(
      this.runtime.session.model?.contextWindow ?? 0,
      this.config.contextLimitTokens,
    );
  }

  private currentEstimatedContextTokens(): number | null {
    const transcript = this.runtime.session.getContextUsage()?.tokens;
    const state = this.runtime.session.agent.state;
    const providerEstimate = estimateProviderContextTokens({
      systemPrompt: state.systemPrompt,
      messages: state.messages,
      tools: state.tools,
    });
    return typeof transcript === "number"
      ? Math.max(transcript, providerEstimate)
      : providerEstimate;
  }

  private getCompactionThreshold(): number {
    return effectiveCompactionSettings(
      this.getEffectiveLimit(),
      this.config.compactionReserveTokens,
      this.config.keepRecentTokens,
    ).thresholdTokens;
  }

  private modelId(): string | null {
    const model = this.runtime.session.model;
    return model ? `${model.provider}/${model.id}` : null;
  }

  private updateUsageSnapshot(): void {
    const stats = this.runtime.session.getSessionStats();
    const input =
      this.offsets.input +
      stats.tokens.input +
      stats.tokens.cacheRead +
      stats.tokens.cacheWrite;
    const output = this.offsets.output + stats.tokens.output;
    const cached = this.offsets.cached + stats.tokens.cacheRead;
    const cost = this.offsets.cost + stats.cost;
    this.lastUsage = {
      contextTokens: stats.contextUsage?.tokens ?? null,
      accumulatedInputTokens: Math.max(
        this.lastUsage.accumulatedInputTokens,
        input,
      ),
      accumulatedOutputTokens: Math.max(
        this.lastUsage.accumulatedOutputTokens,
        output,
      ),
      accumulatedCachedInputTokens: Math.min(
        Math.max(this.lastUsage.accumulatedCachedInputTokens, cached),
        Math.max(this.lastUsage.accumulatedInputTokens, input),
      ),
      accumulatedCost: Math.max(this.lastUsage.accumulatedCost ?? 0, cost),
      model: this.modelId(),
    };
  }

  private clearPendingCompaction(): void {
    this.pendingCompactionReason = undefined;
    this.pendingCompactionBefore = null;
    this.pendingCompactionId = undefined;
    this.pendingCompactionAttemptEntryId = undefined;
  }

  private installFinalProviderGuard(): void {
    const agent = this.runtime.session.agent;
    if (this.providerGuardedAgent === agent) return;
    this.providerGuardedAgent = agent;
    const priorStream = agent.streamFunction;
    agent.streamFunction = async (model, context, options) => {
      const priorOnPayload = options?.onPayload;
      const guardedOptions = {
        ...(options ?? {}),
        onPayload: async (payload: unknown, payloadModel: typeof model) => {
          try {
            return await applyStrictPayloadGuard(
              payload,
              payloadModel,
              priorOnPayload,
              this.getEffectiveLimit(),
            );
          } catch (error) {
            this.emitContextLimitFailure(error);
            throw error;
          }
        },
      };
      try {
        return await guardProviderDispatch(
          context,
          this.getEffectiveLimit(),
          () => priorStream(model, context, guardedOptions),
        );
      } catch (error) {
        this.emitContextLimitFailure(error);
        throw error;
      }
    };
  }

  private emitContextLimitFailure(error: unknown): void {
    if (!isContextLimitError(error) || this.contextLimitFailureEmitted) return;
    this.contextLimitFailureEmitted = true;
    this.pendingCompactionReason = "preflight";
    this.pendingCompactionBefore = this.currentEstimatedContextTokens();
    this.pendingCompactionId = randomUUID();
    this.emitCompactionFailure(
      "preflight",
      errorMessage(error),
      false,
      false,
      false,
    );
    this.clearPendingCompaction();
  }

  private lastCompactionFromExtension(): boolean {
    const entry = this.latestPersistedCompaction();
    return entry?.type === "compaction" && entry.fromHook === true;
  }

  private latestPersistedCompaction(): CompactionEntry | undefined {
    const entry = this.runtime.session.sessionManager
      .getEntries()
      .findLast((candidate) => candidate.type === "compaction");
    return entry?.type === "compaction" ? entry : undefined;
  }

  private persistCompactionLifecycleEvent(
    compactionEntry: CompactionEntry,
    event: BuzzSessionEvent,
  ): string {
    this.ensureLifecycleIndexLoaded();
    const deliveryId = stableLifecycleUuid(
      "delivery",
      this.piSessionId,
      compactionEntry.id,
    );
    const existing = this.pendingLifecycleEvents.get(deliveryId);
    if (existing) {
      if (
        existing.sourceEntryId !== compactionEntry.id ||
        JSON.stringify(existing.event) !== JSON.stringify(event)
      ) {
        throw new Error("conflicting durable Pi lifecycle delivery marker");
      }
      return deliveryId;
    }
    if (this.acknowledgedLifecycleEvents.has(deliveryId)) return deliveryId;
    if (
      this.pendingLifecycleEvents.size >= this.config.maxPendingSessionEvents
    ) {
      throw new Error(
        `Pi child lifecycle outbox capacity ${this.config.maxPendingSessionEvents} is full`,
      );
    }
    const data = {
      version: 1,
      deliveryId,
      sourceEntryId: compactionEntry.id,
      event,
    };
    if (
      Buffer.byteLength(JSON.stringify(data), "utf8") >
      MAX_CHILD_LIFECYCLE_RECORD_BYTES
    ) {
      throw new Error(
        "Pi child lifecycle event exceeds its durable record bound",
      );
    }
    this.runtime.session.sessionManager.appendCustomEntry(
      LIFECYCLE_PENDING_ENTRY,
      data,
    );
    this.lifecycleIndexedLeafId =
      this.runtime.session.sessionManager.getLeafId();
    this.pendingLifecycleEvents.set(deliveryId, {
      sourceEntryId: compactionEntry.id,
      event,
    });
    return deliveryId;
  }

  private ensureLifecycleIndexLoaded(): void {
    const manager = this.runtime.session.sessionManager;
    const currentLeafId = manager.getLeafId();
    if (
      this.lifecycleIndexLoaded &&
      this.lifecycleIndexedLeafId === currentLeafId
    ) {
      return;
    }
    this.pendingLifecycleEvents.clear();
    this.acknowledgedLifecycleEvents.clear();
    this.lifecycleWatermarkEntryId = undefined;
    const activeBranch = manager.getBranch();
    const watermarkIndex = activeBranch.findIndex((entry) => {
      if (
        entry.type !== "custom" ||
        entry.customType !== LIFECYCLE_WATERMARK_ENTRY
      ) {
        return false;
      }
      parseLifecycleWatermarkData(entry.data);
      this.lifecycleWatermarkEntryId = entry.id;
      return true;
    });
    if (watermarkIndex < 0) {
      this.lifecycleWatermarkEntryId = manager.appendCustomEntry(
        LIFECYCLE_WATERMARK_ENTRY,
        { version: 1 },
      );
      this.lifecycleIndexedLeafId = manager.getLeafId();
      this.lifecycleIndexLoaded = true;
      return;
    }
    let markerCount = 0;
    for (const entry of activeBranch.slice(watermarkIndex)) {
      if (entry.type !== "custom") continue;
      if (
        entry.customType !== LIFECYCLE_WATERMARK_ENTRY &&
        entry.customType !== LIFECYCLE_PENDING_ENTRY &&
        entry.customType !== LIFECYCLE_ACK_ENTRY &&
        entry.customType !== COMPACTION_ATTEMPT_ENTRY
      ) {
        continue;
      }
      markerCount += 1;
      if (markerCount > MAX_CHILD_LIFECYCLE_MARKERS) {
        throw new Error("Pi child lifecycle marker count exceeds safe bounds");
      }
      if (entry.customType === LIFECYCLE_PENDING_ENTRY) {
        const pending = parsePendingLifecycleData(entry.data, this.piSessionId);
        const existing = this.pendingLifecycleEvents.get(pending.deliveryId);
        if (
          existing &&
          (existing.sourceEntryId !== pending.sourceEntryId ||
            JSON.stringify(existing.event) !== JSON.stringify(pending.event))
        ) {
          throw new Error("conflicting Pi child lifecycle pending markers");
        }
        this.pendingLifecycleEvents.set(pending.deliveryId, {
          sourceEntryId: pending.sourceEntryId,
          event: pending.event,
        });
      } else if (entry.customType === LIFECYCLE_ACK_ENTRY) {
        this.acknowledgedLifecycleEvents.add(parseLifecycleAckData(entry.data));
      } else if (entry.customType === LIFECYCLE_WATERMARK_ENTRY) {
        parseLifecycleWatermarkData(entry.data);
      }
    }
    for (const deliveryId of this.acknowledgedLifecycleEvents) {
      this.pendingLifecycleEvents.delete(deliveryId);
    }
    if (
      this.pendingLifecycleEvents.size > this.config.maxPendingSessionEvents
    ) {
      throw new Error("Pi child lifecycle outbox exceeds configured capacity");
    }
    this.lifecycleIndexedLeafId = manager.getLeafId();
    this.lifecycleIndexLoaded = true;
  }

  private reconcileMissingCompactionEvents(): void {
    this.ensureLifecycleIndexLoaded();
    const watermarkIndex = this.runtime.session.sessionManager
      .getBranch()
      .findIndex((entry) => entry.id === this.lifecycleWatermarkEntryId);
    if (watermarkIndex < 0) {
      throw new Error(
        "Pi lifecycle watermark is absent from the active branch",
      );
    }
    const compactions = this.runtime.session.sessionManager
      .getBranch()
      .slice(watermarkIndex + 1)
      .filter((entry): entry is CompactionEntry => entry.type === "compaction");
    for (const entry of compactions) {
      const deliveryId = stableLifecycleUuid(
        "delivery",
        this.piSessionId,
        entry.id,
      );
      if (
        this.pendingLifecycleEvents.has(deliveryId) ||
        this.acknowledgedLifecycleEvents.has(deliveryId)
      ) {
        continue;
      }
      const attempt = this.findCompactionAttempt(entry);
      const beforeTokens = Number.isFinite(entry.tokensBefore)
        ? Math.max(0, Math.floor(entry.tokensBefore))
        : null;
      const recovered: BuzzSessionEvent = {
        type: "compaction_completed",
        compactionId:
          attempt?.compactionId ??
          stableLifecycleUuid("compaction", this.piSessionId, entry.id),
        timestamp: entry.timestamp,
        message: `${contextMessage(
          "compacted",
          beforeTokens,
          null,
        )} This notice was recovered from Pi's durable transcript after its runtime restarted.`,
        piSessionId: this.piSessionId,
        reason: attempt?.reason ?? "threshold",
        beforeTokens,
        afterTokens: null,
        limitTokens: this.config.contextLimitTokens,
        effectiveLimitTokens: this.getEffectiveLimit(),
        compactionThresholdTokens: this.getCompactionThreshold(),
        willRetry: false,
        fromExtension: entry.fromHook === true,
      };
      this.persistCompactionLifecycleEvent(entry, recovered);
    }
  }

  private findCompactionAttempt(
    compactionEntry: CompactionEntry,
  ): { compactionId: string; reason: CompactionReason } | undefined {
    let entryId = compactionEntry.parentId;
    let traversed = 0;
    while (entryId !== null && traversed < 10_000) {
      traversed += 1;
      const entry = this.runtime.session.sessionManager.getEntry(entryId);
      if (!entry || entry.type === "compaction") return undefined;
      if (
        entry.type === "custom" &&
        entry.customType === COMPACTION_ATTEMPT_ENTRY
      ) {
        return parseCompactionAttemptData(entry.data);
      }
      entryId = entry.parentId;
    }
    if (traversed >= 10_000) {
      throw new Error("Pi compaction ancestry exceeds safe lifecycle bounds");
    }
    return undefined;
  }

  private resetLifecycleIndex(): void {
    this.lifecycleIndexLoaded = false;
    this.lifecycleIndexedLeafId = undefined;
    this.lifecycleWatermarkEntryId = undefined;
    this.pendingLifecycleEvents.clear();
    this.acknowledgedLifecycleEvents.clear();
  }

  private poisonAfterReloadFailure(cause: unknown): Error {
    if (this.failure) return this.failure;
    const detail = publicError(errorMessage(cause));
    this.failure = new Error(
      `BUZZ_PI_SESSION_INVALIDATED: Pi resource reload failed; create a fresh session (${detail})`,
      { cause },
    );
    this.unsubscribe?.();
    this.unsubscribe = undefined;
    this.busy = false;
    // Start teardown before the failed request returns. dispose() joins this
    // promise, so the runtime host cannot release/recreate the session while
    // the rejected extension generation can still execute callbacks.
    this.fatalDisposal = this.disposeRuntime().catch((error: unknown) => {
      this.logger.error(
        "failed to dispose invalidated Pi resource generation",
        {
          sessionId: this.options.acpSessionId,
          error: publicError(errorMessage(error)),
        },
      );
      throw error;
    });
    // A caller may observe invalidation without immediately disposing the
    // handle. Attach a handler now so a teardown error cannot become an
    // unhandled rejection; dispose() still receives the original rejection.
    void this.fatalDisposal.catch(() => {});
    return this.failure;
  }

  private async disposeRuntime(): Promise<void> {
    if (this.runtime.session.isStreaming || this.runtime.session.isCompacting) {
      await this.runtime.session.abort();
    }
    await this.runtime.dispose();
  }

  private assertUsable(): void {
    if (this.failure) throw this.failure;
    if (this.disposed) throw new Error("Pi session has been disposed");
  }
}

function stableLifecycleUuid(
  kind: "delivery" | "compaction",
  piSessionId: string,
  sourceEntryId: string,
): string {
  const bytes = createHash("sha256")
    .update(`buzz-pi-lifecycle-v1\0${kind}\0${piSessionId}\0${sourceEntryId}`)
    .digest()
    .subarray(0, 16);
  bytes[6] = ((bytes[6] ?? 0) & 0x0f) | 0x50;
  bytes[8] = ((bytes[8] ?? 0) & 0x3f) | 0x80;
  const hex = bytes.toString("hex");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(
    12,
    16,
  )}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

function parsePendingLifecycleData(
  value: unknown,
  expectedPiSessionId: string,
): { deliveryId: string; sourceEntryId: string; event: BuzzSessionEvent } {
  if (!isPlainRecord(value) || value.version !== 1) {
    throw new Error("invalid Pi child lifecycle pending marker");
  }
  if (
    Buffer.byteLength(JSON.stringify(value), "utf8") >
    MAX_CHILD_LIFECYCLE_RECORD_BYTES
  ) {
    throw new Error("Pi child lifecycle pending marker exceeds its byte bound");
  }
  const deliveryId = requiredLifecycleString(
    value.deliveryId,
    "deliveryId",
    64,
  );
  if (!LOWERCASE_UUID_PATTERN.test(deliveryId)) {
    throw new Error("invalid Pi child lifecycle deliveryId");
  }
  const sourceEntryId = requiredLifecycleString(
    value.sourceEntryId,
    "sourceEntryId",
    256,
  );
  assertPersistedCompactionEvent(value.event, expectedPiSessionId);
  return { deliveryId, sourceEntryId, event: value.event };
}

function parseLifecycleAckData(value: unknown): string {
  if (!isPlainRecord(value) || value.version !== 1) {
    throw new Error("invalid Pi child lifecycle acknowledgement marker");
  }
  if (
    Buffer.byteLength(JSON.stringify(value), "utf8") >
    MAX_CHILD_LIFECYCLE_ACK_BYTES
  ) {
    throw new Error(
      "Pi child lifecycle acknowledgement exceeds its byte bound",
    );
  }
  const deliveryId = requiredLifecycleString(
    value.deliveryId,
    "deliveryId",
    64,
  );
  if (!LOWERCASE_UUID_PATTERN.test(deliveryId)) {
    throw new Error("invalid Pi child lifecycle acknowledgement id");
  }
  return deliveryId;
}

function parseLifecycleWatermarkData(value: unknown): void {
  if (
    !isPlainRecord(value) ||
    value.version !== 1 ||
    Object.keys(value).length !== 1 ||
    Buffer.byteLength(JSON.stringify(value), "utf8") >
      MAX_CHILD_LIFECYCLE_WATERMARK_BYTES
  ) {
    throw new Error("invalid Pi child lifecycle watermark");
  }
}

function parseCompactionAttemptData(value: unknown): {
  compactionId: string;
  reason: CompactionReason;
} {
  if (!isPlainRecord(value) || value.version !== 1) {
    throw new Error("invalid Pi compaction attempt marker");
  }
  if (
    Buffer.byteLength(JSON.stringify(value), "utf8") >
    MAX_CHILD_LIFECYCLE_ATTEMPT_BYTES
  ) {
    throw new Error("Pi compaction attempt marker exceeds its byte bound");
  }
  const compactionId = requiredLifecycleString(
    value.compactionId,
    "compactionId",
    64,
  );
  if (!LOWERCASE_UUID_PATTERN.test(compactionId)) {
    throw new Error("invalid Pi compaction attempt id");
  }
  if (
    value.reason !== "manual" &&
    value.reason !== "threshold" &&
    value.reason !== "overflow" &&
    value.reason !== "preflight"
  ) {
    throw new Error("invalid Pi compaction attempt reason");
  }
  assertNullableLifecycleCount(value.beforeTokens, "attempt beforeTokens");
  const startedAt = requiredLifecycleString(
    value.startedAt,
    "attempt startedAt",
    64,
  );
  if (!Number.isFinite(Date.parse(startedAt))) {
    throw new Error("invalid Pi compaction attempt timestamp");
  }
  return { compactionId, reason: value.reason };
}

function assertPersistedCompactionEvent(
  value: unknown,
  expectedPiSessionId: string,
): asserts value is Extract<
  BuzzSessionEvent,
  { type: "compaction_completed" }
> {
  if (!isPlainRecord(value) || value.type !== "compaction_completed") {
    throw new Error("invalid persisted Pi compaction lifecycle event");
  }
  const compactionId = requiredLifecycleString(
    value.compactionId,
    "compactionId",
    64,
  );
  if (!LOWERCASE_UUID_PATTERN.test(compactionId)) {
    throw new Error("invalid persisted Pi compaction id");
  }
  const timestamp = requiredLifecycleString(value.timestamp, "timestamp", 64);
  if (!Number.isFinite(Date.parse(timestamp))) {
    throw new Error("invalid persisted Pi lifecycle timestamp");
  }
  requiredLifecycleString(value.message, "message", 1_024);
  if (value.piSessionId !== expectedPiSessionId) {
    throw new Error("persisted Pi lifecycle generation mismatch");
  }
  if (
    value.reason !== "manual" &&
    value.reason !== "threshold" &&
    value.reason !== "overflow" &&
    value.reason !== "preflight"
  ) {
    throw new Error("invalid persisted Pi lifecycle reason");
  }
  assertNullableLifecycleCount(value.beforeTokens, "beforeTokens");
  assertNullableLifecycleCount(value.afterTokens, "afterTokens");
  assertLifecycleCount(value.limitTokens, "limitTokens", true);
  assertLifecycleCount(
    value.effectiveLimitTokens,
    "effectiveLimitTokens",
    true,
  );
  assertLifecycleCount(
    value.compactionThresholdTokens,
    "compactionThresholdTokens",
    true,
  );
  if (
    typeof value.willRetry !== "boolean" ||
    typeof value.fromExtension !== "boolean"
  ) {
    throw new Error("invalid persisted Pi lifecycle flags");
  }
}

function assertNullableLifecycleCount(value: unknown, name: string): void {
  if (value === null) return;
  assertLifecycleCount(value, name, false);
}

function assertLifecycleCount(
  value: unknown,
  name: string,
  positive: boolean,
): void {
  if (!Number.isSafeInteger(value) || (value as number) < (positive ? 1 : 0)) {
    throw new Error(`invalid persisted Pi lifecycle ${name}`);
  }
}

function requiredLifecycleString(
  value: unknown,
  name: string,
  maxLength: number,
): string {
  if (
    typeof value !== "string" ||
    value.length < 1 ||
    value.length > maxLength ||
    containsControlCharacter(value)
  ) {
    throw new Error(`invalid Pi lifecycle ${name}`);
  }
  return value;
}

function containsControlCharacter(value: string): boolean {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code <= 0x1f || code === 0x7f) return true;
  }
  return false;
}

function isPlainRecord(value: unknown): value is Record<string, unknown> {
  return (
    typeof value === "object" &&
    value !== null &&
    !Array.isArray(value) &&
    (Object.getPrototypeOf(value) === Object.prototype ||
      Object.getPrototypeOf(value) === null)
  );
}

function resolveProjectTrust(
  cwd: string,
  agentDir: string,
  config: AdapterConfig,
  logger: Logger,
): boolean {
  if (config.trustProjectOverride !== undefined) {
    logger.warn("using explicit Buzz Pi project-trust override", {
      cwd,
      trusted: config.trustProjectOverride,
    });
    return config.trustProjectOverride;
  }
  try {
    // Headless Buzz cannot ask an interactive trust question. Only a saved Pi
    // trust decision (or the explicit env override above) enables *any*
    // project-local executable/config resource. This also closes reload TOCTOU
    // when a .pi/extensions directory appears after startup.
    return new ProjectTrustStore(agentDir).get(cwd) === true;
  } catch (error) {
    logger.warn("Pi project trust resolution failed closed", {
      cwd,
      error: errorMessage(error),
    });
    return false;
  }
}

interface ProviderContextLike {
  systemPrompt?: unknown;
  messages?: unknown;
  tools?: unknown;
}

/**
 * Estimate the complete provider-facing context, including the system prompt
 * and tool schemas that Pi's transcript-only accounting does not include.
 */
export function estimateProviderContextTokens(
  context: ProviderContextLike,
): number {
  let tokens = 0;
  if (typeof context.systemPrompt === "string") {
    tokens += estimateAdaptiveTextTokens(context.systemPrompt);
  }
  if (Array.isArray(context.messages)) {
    for (const message of context.messages) {
      // Bound custom-provider structures before Pi's estimator sees them.
      const serializedEstimate = estimateProviderPayloadTokens(message);
      const piEstimate = estimateTokens(
        message as Parameters<typeof estimateTokens>[0],
      );
      tokens += Math.max(piEstimate, serializedEstimate);
      // Provider role/content framing has a small but non-zero cost.
      tokens += 8;
    }
  }
  if (Array.isArray(context.tools) && context.tools.length > 0) {
    tokens +=
      estimateProviderPayloadTokens(context.tools, true) +
      context.tools.length * 16;
  }
  return tokens;
}

export function estimateSerializedPayloadTokens(serialized: string): number {
  return estimateAdaptiveTextTokens(serialized);
}

function estimateAdaptiveTextTokens(text: string): number {
  const bytes = Buffer.byteLength(text, "utf8");
  const base = Math.ceil(bytes / 4);
  const punctuation = text.match(/[^\p{L}\p{N}\s]/gu)?.length ?? 0;
  const asciiSegments = text.match(/[A-Za-z0-9]+|[^\sA-Za-z0-9]/g)?.length ?? 0;
  const nonAsciiBytes = Buffer.byteLength(
    [...text]
      .filter((character) => (character.codePointAt(0) ?? 0) > 0x7f)
      .join(""),
    "utf8",
  );
  const longestDenseRun = text
    .split(/\s+/u)
    .reduce(
      (longest, part) => Math.max(longest, Buffer.byteLength(part, "utf8")),
      0,
    );
  // Pi's chars/4 estimate is accurate for ordinary prose. These additional
  // floors protect token-dense source, punctuation, CJK, emoji, and opaque
  // identifiers that can tokenize materially above that average.
  return Math.max(
    base,
    asciiSegments,
    Math.ceil(punctuation / 2),
    Math.ceil(nonAsciiBytes / 2),
    Math.ceil(longestDenseRun / 2),
  );
}

export function assertWithinContextLimit(
  estimatedTokens: number,
  limitTokens: number,
  source = "provider request",
): void {
  if (!Number.isSafeInteger(estimatedTokens) || estimatedTokens < 0) {
    throw new Error(`BUZZ_CONTEXT_LIMIT: could not safely estimate ${source}`);
  }
  if (estimatedTokens > limitTokens) {
    throw new Error(
      `BUZZ_CONTEXT_LIMIT: ${source} was estimated at ${estimatedTokens} tokens, above the ${limitTokens}-token effective limit; compact or shorten the prompt`,
    );
  }
}

export function guardProviderDispatch<T>(
  context: ProviderContextLike,
  limitTokens: number,
  dispatch: () => T,
): T {
  assertWithinContextLimit(
    estimateProviderContextTokens(context),
    limitTokens,
    "final Pi provider context",
  );
  return dispatch();
}

export async function applyStrictPayloadGuard<TModel>(
  payload: unknown,
  model: TModel,
  priorOnPayload:
    | ((
        payload: unknown,
        model: TModel,
      ) => unknown | undefined | Promise<unknown | undefined>)
    | undefined,
  limitTokens: number,
): Promise<unknown> {
  const initialEstimate = estimateProviderPayloadTokens(payload);
  assertWithinContextLimit(
    initialEstimate,
    limitTokens,
    "serialized Pi provider payload",
  );
  const before = serializeProviderPayload(payload);
  const replacement = priorOnPayload
    ? await priorOnPayload(payload, model)
    : undefined;
  const finalPayload = replacement ?? payload;
  const finalEstimate = estimateProviderPayloadTokens(finalPayload);
  const after = serializeProviderPayload(finalPayload);
  // A raw provider hook runs after Pi has assembled the request. There is no
  // provider-independent tokenizer at this point, so accepting a mutation
  // would make the cap unenforceable. Observation-only hooks remain compatible.
  if (after !== before) {
    throw new Error(
      "BUZZ_CONTEXT_LIMIT: Pi before_provider_request payload mutation is disabled by the strict context cap",
    );
  }
  assertWithinContextLimit(
    finalEstimate,
    limitTokens,
    "final serialized Pi provider payload",
  );
  return finalPayload;
}

/** Estimate text/schema tokens while accounting for image binary separately. */
export function estimateProviderPayloadTokens(
  payload: unknown,
  allowToolFunctions = false,
): number {
  const analysis = {
    images: 0,
    imageBytes: 0,
    nodes: 0,
    entries: 0,
    textualBytes: 0,
    seen: new WeakSet<object>(),
  };
  const inspect = (value: unknown, depth: number): unknown => {
    analysis.nodes += 1;
    if (analysis.nodes > MAX_PROVIDER_PAYLOAD_NODES) {
      throwProviderPayloadBounds("node count");
    }
    if (depth > 32) {
      throw new Error(
        "BUZZ_CONTEXT_LIMIT: provider payload nesting is too deep to inspect",
      );
    }
    if (typeof value === "string" && value.startsWith("data:image/")) {
      recordImage(
        analysis,
        encodedImageBytes(value.slice(value.indexOf(",") + 1)),
      );
      recordProviderText(analysis, "<image-binary>");
      return "<image-binary>";
    }
    if (value === null) {
      recordProviderText(analysis, "null");
      return value;
    }
    if (typeof value === "string") {
      recordProviderText(analysis, value);
      return value;
    }
    if (typeof value === "number") {
      if (!Number.isFinite(value)) throwUnsafeProviderPayload();
      recordProviderText(analysis, String(value));
      return value;
    }
    if (typeof value === "boolean") {
      recordProviderText(analysis, String(value));
      return value;
    }
    if (value === undefined) {
      // Pi's own JSON-shaped message/tool types use optional properties with
      // undefined values. JSON serialization omits those object fields (or
      // emits null for an array slot), so retaining undefined here mirrors the
      // real provider payload while the surrounding structure remains bounded.
      recordProviderText(analysis, "undefined");
      return value;
    }
    if (
      allowToolFunctions &&
      (typeof value === "function" || typeof value === "symbol")
    ) {
      // Agent tool objects contain local execute/render functions that are
      // removed when Pi builds the provider-facing JSON schema.
      return undefined;
    }
    if (typeof value !== "object") throwUnsafeProviderPayload();
    if (!Array.isArray(value) && !isPlainObject(value)) {
      throw new Error(
        "BUZZ_CONTEXT_LIMIT: provider payload contains non-plain JSON data",
      );
    }
    if (analysis.seen.has(value)) {
      throwUnsafeProviderPayload();
    }
    analysis.seen.add(value);
    if (Array.isArray(value)) {
      assertProviderArrayShape(value);
      const result: unknown[] = [];
      for (let index = 0; index < value.length; index += 1) {
        recordProviderEntry(analysis);
        let descriptor: PropertyDescriptor | undefined;
        try {
          descriptor = Object.getOwnPropertyDescriptor(value, index);
        } catch {
          throwUnsafeProviderPayload();
        }
        if (!descriptor || !("value" in descriptor)) {
          throwUnsafeProviderPayload();
        }
        result.push(inspect(descriptor.value, depth + 1));
      }
      return result;
    }
    const record = value as Record<string, unknown>;
    assertNoProviderSymbolKeys(record);
    const type = providerOwnDataValue(record, "type");
    const mimeType = providerOwnDataValue(record, "mimeType");
    const mimeTypeSnake = providerOwnDataValue(record, "mime_type");
    const data = providerOwnDataValue(record, "data");
    const imageContainer =
      type === "image" ||
      type === "input_image" ||
      type === "image_url" ||
      type === "base64" ||
      ((typeof mimeType === "string" || typeof mimeTypeSnake === "string") &&
        typeof data === "string");
    const result: Record<string, unknown> = {};
    let enumeratedKeys = 0;
    try {
      for (const key in record) {
        enumeratedKeys += 1;
        if (enumeratedKeys > MAX_PROVIDER_PAYLOAD_ENTRIES) {
          throwProviderPayloadBounds("enumerated keys");
        }
        if (!Object.hasOwn(record, key)) continue;
        recordProviderEntry(analysis);
        recordProviderText(analysis, key);
        const item = providerOwnDataValue(record, key);
        if (imageContainer && key === "data" && typeof item === "string") {
          recordImage(analysis, encodedImageBytes(item));
          recordProviderText(analysis, "<image-binary>");
          result[key] = "<image-binary>";
        } else if (
          (key === "image_url" || key === "url") &&
          typeof item === "string" &&
          item.startsWith("data:image/")
        ) {
          recordImage(
            analysis,
            encodedImageBytes(item.slice(item.indexOf(",") + 1)),
          );
          recordProviderText(analysis, "<image-binary>");
          result[key] = "<image-binary>";
        } else {
          result[key] = inspect(item, depth + 1);
        }
      }
    } catch (error) {
      if (isProviderPayloadError(error)) throw error;
      throwUnsafeProviderPayload();
    }
    return result;
  };
  const textualPayload = serializeProviderPayload(inspect(payload, 0));
  return (
    estimateSerializedPayloadTokens(textualPayload) +
    analysis.images * ESTIMATED_PROVIDER_IMAGE_TOKENS
  );
}

function recordProviderEntry(analysis: { entries: number }): void {
  analysis.entries += 1;
  if (analysis.entries > MAX_PROVIDER_PAYLOAD_ENTRIES) {
    throwProviderPayloadBounds("entry count");
  }
}

function recordProviderText(
  analysis: { textualBytes: number },
  value: string,
): void {
  const remaining = MAX_PROVIDER_PAYLOAD_TEXT_BYTES - analysis.textualBytes;
  if (value.length > remaining) throwProviderPayloadBounds("text bytes");
  analysis.textualBytes += Buffer.byteLength(value);
  if (analysis.textualBytes > MAX_PROVIDER_PAYLOAD_TEXT_BYTES) {
    throwProviderPayloadBounds("text bytes");
  }
}

function throwProviderPayloadBounds(kind: string): never {
  throw new Error(
    `BUZZ_CONTEXT_LIMIT: provider payload ${kind} exceeds safe structural bounds`,
  );
}

function throwUnsafeProviderPayload(): never {
  throw new Error(
    "BUZZ_CONTEXT_LIMIT: provider payload could not be safely inspected",
  );
}

function isPlainObject(value: object): boolean {
  try {
    const prototype = Object.getPrototypeOf(value);
    return prototype === Object.prototype || prototype === null;
  } catch {
    return false;
  }
}

function assertProviderArrayShape(value: unknown[]): void {
  if (value.length > MAX_PROVIDER_PAYLOAD_ENTRIES) {
    throwProviderPayloadBounds("array length");
  }
  assertNoProviderSymbolKeys(value);
  let enumeratedKeys = 0;
  let ownEnumerableKeys = 0;
  try {
    for (const key in value) {
      enumeratedKeys += 1;
      if (enumeratedKeys > MAX_PROVIDER_PAYLOAD_ENTRIES) {
        throwProviderPayloadBounds("enumerated keys");
      }
      if (!Object.hasOwn(value, key)) continue;
      if (
        ownEnumerableKeys >= value.length ||
        key !== String(ownEnumerableKeys)
      ) {
        throw new Error(
          "BUZZ_CONTEXT_LIMIT: provider payload contains a decorated array",
        );
      }
      ownEnumerableKeys += 1;
    }
  } catch (error) {
    if (isProviderPayloadError(error)) throw error;
    throwUnsafeProviderPayload();
  }
  if (ownEnumerableKeys !== value.length) {
    throw new Error(
      "BUZZ_CONTEXT_LIMIT: provider payload contains a sparse array",
    );
  }
}

function assertNoProviderSymbolKeys(value: object): void {
  let symbols: symbol[];
  try {
    symbols = Object.getOwnPropertySymbols(value);
  } catch {
    throwUnsafeProviderPayload();
  }
  if (symbols.length > 0) {
    throw new Error(
      "BUZZ_CONTEXT_LIMIT: provider payload contains symbol-keyed data",
    );
  }
}

function providerOwnDataValue(
  value: Record<string, unknown>,
  key: string,
): unknown {
  let descriptor: PropertyDescriptor | undefined;
  try {
    descriptor = Object.getOwnPropertyDescriptor(value, key);
  } catch {
    throwUnsafeProviderPayload();
  }
  if (!descriptor) return undefined;
  if (!("value" in descriptor)) throwUnsafeProviderPayload();
  return descriptor.value;
}

function isProviderPayloadError(error: unknown): error is Error {
  return (
    error instanceof Error && error.message.startsWith("BUZZ_CONTEXT_LIMIT:")
  );
}

function recordImage(
  analysis: { images: number; imageBytes: number },
  bytes: number,
): void {
  analysis.images++;
  analysis.imageBytes += bytes;
  if (
    analysis.images > MAX_PROVIDER_IMAGES ||
    bytes > MAX_PROVIDER_IMAGE_BYTES ||
    analysis.imageBytes > MAX_PROVIDER_IMAGE_BYTES_TOTAL
  ) {
    throw new Error(
      "BUZZ_CONTEXT_LIMIT: provider image count or size exceeds safe bounds",
    );
  }
}

function encodedImageBytes(value: string): number {
  const padding = value.endsWith("==") ? 2 : value.endsWith("=") ? 1 : 0;
  return Math.max(0, Math.floor((value.length * 3) / 4) - padding);
}

function serializeProviderPayload(payload: unknown): string {
  let serialized: string | undefined;
  try {
    serialized = JSON.stringify(payload);
  } catch {
    throw new Error(
      "BUZZ_CONTEXT_LIMIT: provider payload could not be safely inspected",
    );
  }
  if (serialized === undefined) {
    throw new Error(
      "BUZZ_CONTEXT_LIMIT: provider payload could not be safely inspected",
    );
  }
  if (Buffer.byteLength(serialized) > MAX_PROVIDER_SERIALIZED_BYTES) {
    throwProviderPayloadBounds("serialized bytes");
  }
  return serialized;
}

function limitModelContext(
  model: PiModelLike,
  config: AdapterConfig,
): PiModelLike {
  return {
    ...model,
    contextWindow: logicalModelContextWindow(
      model.contextWindow,
      config.contextLimitTokens,
    ),
  };
}

function resolveModel(
  runtime: ModelRuntime,
  modelId: string,
): PiModelLike | undefined {
  const models = runtime.getAvailableSnapshot();
  const exact = models.filter(
    (model) =>
      `${model.provider}/${model.id}` === modelId || model.id === modelId,
  );
  return exact.length === 1 ? exact[0] : undefined;
}

export function applyFreshSessionTitle(
  session: { setSessionName(name: string): void },
  title: string | undefined,
): void {
  const normalized = title?.trim();
  if (!normalized) return;
  session.setSessionName(boundedString(normalized, MAX_SESSION_TITLE_LENGTH));
}

function persistFreshSession(sessionManager: SessionManager): void {
  // Pi 0.83 intentionally delays writing a session until its first assistant
  // message. Buzz needs a durable thread mapping immediately at session/new.
  // This exact, version-pinned seam writes the existing header/title entries
  // and marks them flushed so Pi appends normally on the first turn.
  const pinned = sessionManager as unknown as {
    _rewriteFile?: () => void;
    flushed?: boolean;
  };
  if (typeof pinned._rewriteFile !== "function") {
    throw new Error(
      "Installed Pi SDK is incompatible with durable Buzz sessions",
    );
  }
  pinned._rewriteFile();
  pinned.flushed = true;
}

/**
 * Install a byte-accurate guard at Pi's version-pinned append/rewrite seam.
 *
 * Pi transcripts are append-only: context compaction reduces provider context
 * but does not reclaim JSONL bytes. Ordinary and Buzz control entries are
 * accounted in separate fixed partitions. Rollback/lifecycle records can use
 * their reserve without stealing ordinary transcript headroom, while neither
 * partition nor their combined bytes can cross the configured hard ceiling.
 */
export function installSessionFileQuota(
  sessionManager: SessionManager,
  maxBytes: number,
): void {
  if (quotaInstalled.has(sessionManager)) return;
  if (!Number.isSafeInteger(maxBytes) || maxBytes <= 0) {
    throw new Error("invalid Pi session transcript byte limit");
  }
  const pinned = sessionManager as unknown as {
    _appendEntry?: (entry: unknown) => void;
    _rewriteFile?: () => void;
    fileEntries?: unknown[];
  };
  if (
    typeof pinned._appendEntry !== "function" ||
    typeof pinned._rewriteFile !== "function" ||
    !Array.isArray(pinned.fileEntries)
  ) {
    throw new Error(
      "Installed Pi SDK is incompatible with the Buzz transcript quota",
    );
  }
  const originalAppend = pinned._appendEntry;
  const originalRewrite = pinned._rewriteFile;
  const controlReserveBytes = Math.min(
    SESSION_FILE_CONTROL_RESERVE_BYTES,
    maxBytes,
  );
  const ordinaryLimitBytes = maxBytes - controlReserveBytes;
  let trackedEntries = pinned.fileEntries;
  let accounting = classifySessionEntries(trackedEntries);
  const refreshAccounting = (): void => {
    if (!Array.isArray(pinned.fileEntries)) {
      throw new Error(
        "Installed Pi SDK changed its transcript storage representation",
      );
    }
    if (pinned.fileEntries !== trackedEntries) {
      trackedEntries = pinned.fileEntries;
      accounting = classifySessionEntries(trackedEntries);
    }
  };
  const assertAccountingWithinQuota = (
    nextOrdinaryBytes: number,
    nextControlBytes: number,
  ): void => {
    const totalBytes = nextOrdinaryBytes + nextControlBytes;
    if (
      !Number.isSafeInteger(nextOrdinaryBytes) ||
      !Number.isSafeInteger(nextControlBytes) ||
      nextOrdinaryBytes < 0 ||
      nextControlBytes < 0 ||
      nextOrdinaryBytes > ordinaryLimitBytes ||
      nextControlBytes > controlReserveBytes ||
      !Number.isSafeInteger(totalBytes) ||
      totalBytes > maxBytes
    ) {
      throw sessionStorageLimitError(
        Math.min(Number.MAX_SAFE_INTEGER, Math.max(0, totalBytes)),
        maxBytes,
      );
    }
  };
  assertAccountingWithinQuota(
    accounting.ordinaryBytes,
    accounting.controlBytes,
  );
  pinned._appendEntry = (entry: unknown): void => {
    refreshAccounting();
    const incomingBytes = serializedSessionEntryBytes(entry);
    const control = isBuzzControlEntry(entry);
    if (control) assertBuzzControlEntry(entry, incomingBytes);
    const nextOrdinaryBytes =
      accounting.ordinaryBytes + (control ? 0 : incomingBytes);
    const nextControlBytes =
      accounting.controlBytes + (control ? incomingBytes : 0);
    assertAccountingWithinQuota(nextOrdinaryBytes, nextControlBytes);
    const sessionFile = sessionManager.getSessionFile();
    if (sessionManager.isPersisted() && sessionFile) {
      assertSessionFileGrowthWithinQuota(
        sessionFile,
        incomingBytes,
        maxBytes,
        maxBytes,
      );
    }
    originalAppend.call(sessionManager, entry);
    if (pinned.fileEntries === trackedEntries) {
      accounting = {
        ordinaryBytes: nextOrdinaryBytes,
        controlBytes: nextControlBytes,
      };
    } else {
      refreshAccounting();
    }
  };
  pinned._rewriteFile = (): void => {
    if (!Array.isArray(pinned.fileEntries)) {
      throw new Error(
        "Installed Pi SDK changed its transcript storage representation",
      );
    }
    const rewrittenAccounting = classifySessionEntries(pinned.fileEntries);
    assertAccountingWithinQuota(
      rewrittenAccounting.ordinaryBytes,
      rewrittenAccounting.controlBytes,
    );
    originalRewrite.call(sessionManager);
    trackedEntries = pinned.fileEntries;
    accounting = rewrittenAccounting;
  };
  quotaInstalled.add(sessionManager);
}

export function assertSessionFileSizeWithinQuota(
  sessionFile: string,
  maxBytes: number,
): void {
  let currentBytes: number;
  try {
    currentBytes = statSync(sessionFile).size;
  } catch (error) {
    if (isFileNotFound(error)) return;
    throw error;
  }
  if (currentBytes > maxBytes) {
    throw sessionStorageLimitError(currentBytes, maxBytes);
  }
}

function assertSessionFileGrowthWithinQuota(
  sessionFile: string,
  incomingBytes: number,
  operationLimitBytes: number,
  configuredMaxBytes: number,
): void {
  let currentBytes = 0;
  try {
    currentBytes = statSync(sessionFile).size;
  } catch (error) {
    if (!isFileNotFound(error)) throw error;
  }
  if (
    !Number.isSafeInteger(currentBytes) ||
    !Number.isSafeInteger(incomingBytes) ||
    incomingBytes < 0 ||
    currentBytes > operationLimitBytes - incomingBytes
  ) {
    throw sessionStorageLimitError(
      Math.min(Number.MAX_SAFE_INTEGER, currentBytes + incomingBytes),
      configuredMaxBytes,
    );
  }
}

function serializedSessionEntryBytes(entry: unknown): number {
  let serialized: string | undefined;
  try {
    serialized = JSON.stringify(entry);
  } catch {
    throw new Error(
      "BUZZ_SESSION_STORAGE_LIMIT: Pi attempted to persist an unserializable session entry; use /new to start a clean session",
    );
  }
  if (serialized === undefined) {
    throw new Error(
      "BUZZ_SESSION_STORAGE_LIMIT: Pi attempted to persist an invalid session entry; use /new to start a clean session",
    );
  }
  return Buffer.byteLength(`${serialized}\n`, "utf8");
}

function classifySessionEntries(entries: readonly unknown[]): {
  ordinaryBytes: number;
  controlBytes: number;
} {
  let ordinaryBytes = 0;
  let controlBytes = 0;
  for (const entry of entries) {
    const bytes = serializedSessionEntryBytes(entry);
    if (isBuzzControlEntry(entry)) {
      assertBuzzControlEntry(entry, bytes);
      controlBytes += bytes;
    } else {
      ordinaryBytes += bytes;
    }
  }
  return { ordinaryBytes, controlBytes };
}

function isBuzzControlEntry(value: unknown): boolean {
  return (
    typeof value === "object" &&
    value !== null &&
    "type" in value &&
    value.type === "custom" &&
    "customType" in value &&
    (value.customType === "buzz.turn_rollback" ||
      value.customType === COMPACTION_ATTEMPT_ENTRY ||
      value.customType === LIFECYCLE_WATERMARK_ENTRY ||
      value.customType === LIFECYCLE_PENDING_ENTRY ||
      value.customType === LIFECYCLE_ACK_ENTRY)
  );
}

function assertBuzzControlEntry(value: unknown, serializedBytes: number): void {
  if (!isPlainRecord(value) || value.type !== "custom") {
    throw new Error("invalid Buzz transcript control entry");
  }
  const data = value.data;
  if (value.customType === "buzz.turn_rollback") {
    if (
      serializedBytes > MAX_CHILD_ROLLBACK_MARKER_BYTES ||
      !isPlainRecord(data) ||
      data.version !== 1
    ) {
      throw new Error("invalid bounded Buzz rollback marker");
    }
    return;
  }
  if (value.customType === COMPACTION_ATTEMPT_ENTRY) {
    if (serializedBytes > MAX_CHILD_LIFECYCLE_ATTEMPT_BYTES) {
      throw new Error("Pi compaction attempt entry exceeds its byte bound");
    }
    parseCompactionAttemptData(data);
    return;
  }
  if (value.customType === LIFECYCLE_WATERMARK_ENTRY) {
    if (serializedBytes > MAX_CHILD_LIFECYCLE_WATERMARK_BYTES) {
      throw new Error("Pi lifecycle watermark entry exceeds its byte bound");
    }
    parseLifecycleWatermarkData(data);
    return;
  }
  if (value.customType === LIFECYCLE_ACK_ENTRY) {
    if (serializedBytes > MAX_CHILD_LIFECYCLE_ACK_BYTES) {
      throw new Error("Pi lifecycle ACK entry exceeds its byte bound");
    }
    parseLifecycleAckData(data);
    return;
  }
  if (value.customType === LIFECYCLE_PENDING_ENTRY) {
    if (
      serializedBytes > MAX_CHILD_LIFECYCLE_RECORD_BYTES + 1_024 ||
      !isPlainRecord(data) ||
      !isPlainRecord(data.event) ||
      typeof data.event.piSessionId !== "string"
    ) {
      throw new Error("invalid bounded Pi lifecycle pending entry");
    }
    parsePendingLifecycleData(data, data.event.piSessionId);
    return;
  }
  throw new Error("unknown Buzz transcript control entry");
}

function sessionStorageLimitError(
  currentBytes: number,
  maxBytes: number,
): Error {
  return new Error(
    `BUZZ_SESSION_STORAGE_LIMIT: this Pi thread transcript reached its ${formatBytes(maxBytes)} storage ceiling (${formatBytes(currentBytes)} requested); use /new to start a fresh session`,
  );
}

function formatBytes(bytes: number): string {
  if (bytes >= 1_024 * 1_024)
    return `${(bytes / (1_024 * 1_024)).toFixed(1)} MiB`;
  if (bytes >= 1_024) return `${(bytes / 1_024).toFixed(1)} KiB`;
  return `${bytes} bytes`;
}

export function dedupeCommands<T extends { name: string }>(
  commands: readonly T[],
): T[] {
  const seen = new Set<string>();
  return commands.filter((command) => {
    if (seen.has(command.name)) return false;
    seen.add(command.name);
    return true;
  });
}

function describeModel(model: PiModelLike): ModelDescriptor {
  return {
    id: `${model.provider}/${model.id}`,
    name: boundedString(model.name, 256),
  };
}

function firstLine(text: string): string {
  return text.split(/\r?\n/, 1)[0]?.trim() ?? "";
}

function toolKind(name: string): string {
  if (name === "bash") return "execute";
  if (["read", "grep", "find", "ls"].includes(name)) return "search";
  if (["write", "edit"].includes(name)) return "edit";
  return "other";
}

function truncatePayload(value: unknown): unknown {
  const normalized = normalizeToolEventPayload(value);
  const serialized = JSON.stringify(normalized);
  if (Buffer.byteLength(serialized) <= MAX_TOOL_EVENT_PAYLOAD_BYTES) {
    return normalized;
  }
  const suffix = "\n… output truncated by buzz-pi-agent …";
  return `${truncateUtf8(
    serialized,
    MAX_TOOL_EVENT_PAYLOAD_BYTES - Buffer.byteLength(suffix),
  )}${suffix}`;
}

interface ToolPayloadBudget {
  remainingNodes: number;
  remainingTextCharacters: number;
  ancestors: WeakSet<object>;
}

function normalizeToolEventPayload(value: unknown): unknown {
  return normalizeToolEventNode(value, 0, {
    remainingNodes: MAX_TOOL_EVENT_PAYLOAD_NODES,
    remainingTextCharacters: MAX_TOOL_EVENT_TEXT_CHARACTERS,
    ancestors: new WeakSet(),
  });
}

function normalizeToolEventNode(
  value: unknown,
  depth: number,
  budget: ToolPayloadBudget,
): unknown {
  if (depth > MAX_TOOL_EVENT_DEPTH || budget.remainingNodes <= 0) {
    return "[truncated]";
  }
  budget.remainingNodes -= 1;
  if (typeof value === "string") return takeToolEventString(value, budget);
  if (typeof value === "number") {
    return Number.isFinite(value) ? value : `[${String(value)}]`;
  }
  if (typeof value === "boolean" || value === null) return value;
  if (value === undefined) return "[undefined]";
  if (typeof value === "bigint") {
    return takeToolEventString(`[bigint:${String(value)}]`, budget);
  }
  if (typeof value === "symbol" || typeof value === "function") {
    return `[unsupported:${typeof value}]`;
  }
  if (budget.ancestors.has(value)) return "[circular]";

  if (value instanceof ArrayBuffer) {
    return { $type: "ArrayBuffer", byteLength: value.byteLength };
  }
  if (
    typeof SharedArrayBuffer !== "undefined" &&
    value instanceof SharedArrayBuffer
  ) {
    return { $type: "SharedArrayBuffer", byteLength: value.byteLength };
  }
  if (ArrayBuffer.isView(value)) {
    return {
      $type: value instanceof DataView ? "DataView" : "TypedArray",
      byteLength: value.byteLength,
    };
  }
  if (value instanceof Date) {
    const timestamp = Date.prototype.getTime.call(value);
    return {
      $type: "Date",
      value: Number.isFinite(timestamp)
        ? Date.prototype.toISOString.call(value)
        : "Invalid Date",
    };
  }
  if (value instanceof Error) {
    return {
      $type: "Error",
      name: takeToolEventString(value.name, budget),
      message: takeToolEventString(value.message, budget),
    };
  }

  budget.ancestors.add(value);
  try {
    if (Array.isArray(value)) {
      const count = Math.min(value.length, MAX_TOOL_EVENT_CONTAINER_ITEMS);
      const result: unknown[] = [];
      for (let index = 0; index < count; index += 1) {
        let descriptor: PropertyDescriptor | undefined;
        try {
          descriptor = Object.getOwnPropertyDescriptor(value, index);
        } catch {
          result.push("[unavailable]");
          continue;
        }
        result.push(
          descriptor && "value" in descriptor
            ? normalizeToolEventNode(descriptor.value, depth + 1, budget)
            : descriptor
              ? "[accessor]"
              : "[empty]",
        );
      }
      if (value.length > count) result.push("[truncated]");
      return result;
    }
    if (value instanceof Map) {
      const entries: unknown[] = [];
      let index = 0;
      try {
        for (const [key, item] of Map.prototype.entries.call(value)) {
          if (index >= MAX_TOOL_EVENT_CONTAINER_ITEMS) {
            entries.push("[truncated]");
            break;
          }
          entries.push([
            normalizeToolEventNode(key, depth + 1, budget),
            normalizeToolEventNode(item, depth + 1, budget),
          ]);
          index += 1;
        }
      } catch {
        entries.push("[unavailable]");
      }
      return { $type: "Map", entries };
    }
    if (value instanceof Set) {
      const values: unknown[] = [];
      let index = 0;
      try {
        for (const item of Set.prototype.values.call(value)) {
          if (index >= MAX_TOOL_EVENT_CONTAINER_ITEMS) {
            values.push("[truncated]");
            break;
          }
          values.push(normalizeToolEventNode(item, depth + 1, budget));
          index += 1;
        }
      } catch {
        values.push("[unavailable]");
      }
      return { $type: "Set", values };
    }

    let prototype: object | null;
    try {
      prototype = Object.getPrototypeOf(value);
    } catch {
      return { $type: "NonPlainObject", value: "[unavailable]" };
    }
    const result: Record<string, unknown> = {};
    if (prototype !== Object.prototype && prototype !== null) {
      result.$type = "NonPlainObject";
    }
    let entries = 0;
    let enumeratedKeys = 0;
    try {
      for (const key in value) {
        enumeratedKeys += 1;
        if (enumeratedKeys > MAX_TOOL_EVENT_CONTAINER_ITEMS * 2) {
          result["[truncated]"] = true;
          break;
        }
        if (!Object.hasOwn(value, key)) continue;
        if (entries >= MAX_TOOL_EVENT_CONTAINER_ITEMS) {
          result["[truncated]"] = true;
          break;
        }
        let descriptor: PropertyDescriptor | undefined;
        try {
          descriptor = Object.getOwnPropertyDescriptor(value, key);
        } catch {
          result[key] = "[unavailable]";
          entries += 1;
          continue;
        }
        result[key] =
          descriptor && "value" in descriptor
            ? normalizeToolEventNode(descriptor.value, depth + 1, budget)
            : "[accessor]";
        entries += 1;
      }
    } catch {
      result["[unavailable]"] = true;
    }
    return result;
  } finally {
    budget.ancestors.delete(value);
  }
}

function takeToolEventString(value: string, budget: ToolPayloadBudget): string {
  const allowed = Math.max(
    0,
    Math.min(value.length, budget.remainingTextCharacters),
  );
  const truncated = allowed < value.length;
  const result = truncated
    ? `${value.slice(0, Math.max(0, allowed - 1))}…`
    : value;
  budget.remainingTextCharacters = Math.max(
    0,
    budget.remainingTextCharacters - result.length,
  );
  return result;
}

function truncateUtf8(value: string, maxBytes: number): string {
  let low = 0;
  let high = value.length;
  while (low < high) {
    const middle = Math.ceil((low + high) / 2);
    if (Buffer.byteLength(value.slice(0, middle)) <= maxBytes) low = middle;
    else high = middle - 1;
  }
  return value.slice(0, low);
}

function contextMessage(
  verb: string,
  beforeTokens: number | null,
  afterTokens: number | null,
): string {
  const before =
    beforeTokens === null ? "an unknown size" : formatTokens(beforeTokens);
  const after =
    afterTokens === null ? "a compact summary" : formatTokens(afterTokens);
  return `Pi ${verb} this thread's context from ${before} to approximately ${after}.`;
}

function formatTokens(tokens: number): string {
  return `${new Intl.NumberFormat("en-US").format(Math.round(tokens))} tokens`;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function isContextLimitError(error: unknown): boolean {
  return errorMessage(error).startsWith("BUZZ_CONTEXT_LIMIT:");
}

function isFileNotFound(error: unknown): boolean {
  return (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    error.code === "ENOENT"
  );
}

function boundedString(value: string, maxLength: number): string {
  return value.length <= maxLength ? value : `${value.slice(0, maxLength)}…`;
}

function publicError(value: string): string {
  const withoutWindowsPaths = value.replace(
    /[A-Za-z]:\\(?:[^\\\s:]+\\)*[^\\\s:]*/g,
    "<path>",
  );
  const withoutUnixPaths = withoutWindowsPaths.replace(
    /\/(?:[^/\s:]+\/)*[^/\s:]*/g,
    "<path>",
  );
  return boundedString(withoutUnixPaths, 1_024);
}
