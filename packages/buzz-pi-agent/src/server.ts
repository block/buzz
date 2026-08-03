import { randomUUID } from "node:crypto";
import { isAbsolute } from "node:path";
import type { Readable } from "node:stream";
import type { AdapterConfig } from "./config.js";
import type { IsolatedPiWorkerFactory } from "./runtime-worker.js";
import type { SessionRegistry } from "./session-registry.js";
import type {
  AcpContentBlock,
  AcpImageBlock,
  AdapterEventSink,
  AgentSessionHandle,
  BuzzSessionEvent,
  JsonRpcInbound,
  Logger,
  OutputWriter,
  ResourceSnapshot,
  SessionUsageSnapshot,
} from "./types.js";
import {
  AGENT_CONTEXT_LIMIT,
  AGENT_OVERLOADED,
  AGENT_SESSION_STORAGE_LIMIT,
  AGENT_SESSION_INVALIDATED,
  INTERNAL_ERROR,
  INVALID_PARAMS,
  METHOD_NOT_FOUND,
  JsonRpcError,
  asRecord,
  errorResponse,
  notification,
  parseInbound,
  readNdjson,
  requiredString,
  response,
} from "./wire.js";

const MAX_SYSTEM_PROMPT_BYTES = 512 * 1024;
const MAX_PROMPT_BLOCKS = 128;
const MAX_PROMPT_BYTES = 8 * 1024 * 1024;
const RESERVED_CONTROL_REQUESTS = 16;
const LOWERCASE_UUID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u;

export class AcpServer implements AdapterEventSink {
  private readonly activeRequests = new Set<Promise<void>>();
  private activeOrdinaryRequests = 0;
  private initialized = false;
  private closing = false;
  private shutdownPromise: Promise<void> | undefined;
  private readonly lifecycleTasks = new Map<string, Promise<void>>();
  private readonly lifecycleFailures = new Map<string, Error>();
  private readonly publishedLifecycleEventIds = new Map<string, Set<string>>();

  constructor(
    private readonly input: Readable,
    private readonly output: OutputWriter,
    private readonly registry: SessionRegistry,
    private readonly config: AdapterConfig,
    private readonly logger: Logger,
    private readonly workerFactory?: IsolatedPiWorkerFactory,
  ) {}

  async run(): Promise<void> {
    await this.registry.start();
    try {
      for await (const line of readNdjson(
        this.input,
        this.config.maxLineBytes,
      )) {
        if (line.trim() === "") continue;
        let message: JsonRpcInbound;
        try {
          message = parseInbound(line);
        } catch (error) {
          this.output.write(errorResponse(null, error));
          continue;
        }
        const control = isControlMethod(message.method);
        if (
          (!control &&
            this.activeOrdinaryRequests >= this.config.maxActiveRequests) ||
          this.activeRequests.size >=
            this.config.maxActiveRequests + RESERVED_CONTROL_REQUESTS
        ) {
          if ("id" in message) {
            this.output.write(
              errorResponse(
                message.id,
                new JsonRpcError(
                  AGENT_OVERLOADED,
                  "Pi agent request capacity is full",
                  { kind: "overloaded", retryable: true },
                ),
              ),
            );
          }
          continue;
        }
        if (!control) this.activeOrdinaryRequests += 1;
        const task = this.dispatch(message)
          .catch((error: unknown) => {
            this.logger.warn("ACP notification failed", {
              method: message.method,
              error: error instanceof Error ? error.message : String(error),
            });
          })
          .finally(() => {
            this.activeRequests.delete(task);
            if (!control) this.activeOrdinaryRequests -= 1;
          });
        this.activeRequests.add(task);
      }
    } finally {
      // Start cancellation before waiting for request handlers. A prompt may
      // otherwise remain blocked for the full provider timeout after stdin or
      // the output transport has failed.
      const shutdown = this.shutdown();
      await Promise.allSettled([...this.activeRequests]);
      await shutdown;
    }
  }

  shutdown(): Promise<void> {
    this.shutdownPromise ??= this.performShutdown();
    return this.shutdownPromise;
  }

  private async performShutdown(): Promise<void> {
    this.closing = true;
    await Promise.allSettled([...this.lifecycleTasks.values()]);
    await this.registry.shutdown();
    await Promise.allSettled([...this.lifecycleTasks.values()]);
    await this.workerFactory?.shutdown();
    this.lifecycleTasks.clear();
    this.lifecycleFailures.clear();
    this.publishedLifecycleEventIds.clear();
    await this.output.end();
  }

  sessionUpdate(sessionId: string, update: Record<string, unknown>): void {
    this.output.write(
      notification("session/update", {
        sessionId,
        update: sanitizeValue(update),
      }),
    );
  }

  buzzSessionEvent(
    sessionId: string,
    event: BuzzSessionEvent,
    deliveryId?: string,
  ): void | Promise<void> {
    const safeEvent = sanitizeLifecycleValue(event) as BuzzSessionEvent;
    if (deliveryId !== undefined && !LOWERCASE_UUID_PATTERN.test(deliveryId)) {
      throw new Error(
        "Pi runtime lifecycle deliveryId must be a lowercase UUID",
      );
    }
    const conversationIdentity =
      this.registry.conversationIdentityForSession(sessionId);
    if (!conversationIdentity) {
      // The adapter advertises schema v2 globally during initialize. Emitting
      // an older shape for one session would violate that negotiated contract
      // and make a strict Buzz harness terminate the whole transport. Sessions
      // without a durable Buzz identity still compact normally, but their
      // typed lifecycle notices cannot be safely published or replayed.
      this.logger.debug("suppressed lifecycle event for unmapped ACP session", {
        sessionId: boundedString(sessionId, 256),
        eventType: safeEvent.type,
      });
      return;
    }
    const { conversationId, lifecycleGeneration } = conversationIdentity;
    const eventId = deliveryId ?? randomUUID();
    return this.queueLifecycleTask(sessionId, async () => {
      // Pi extensions can emit during SDK creation, before the manifest route
      // is installed. Keep the child handoff pending until that atomic
      // promotion completes; a rejected create therefore never ACKs the marker.
      await conversationIdentity.readiness;
      const persisted = await this.registry.persistConversationSessionEvent(
        conversationId,
        lifecycleGeneration,
        eventId,
        safeEvent,
      );
      // A concurrent authenticated reset may supersede this Pi generation
      // before its asynchronous outbox write begins. The store fences that
      // race; a rejected stale write must never be published as if durable.
      if (!persisted) return;
      if (conversationIdentity.deferPublication) {
        // session/new will discover this durable outbox record below and emit
        // it via the existing setImmediate replay after the synchronous ACP
        // response. The child may ACK now because the parent copy is durable.
        return;
      }
      if (!this.publishedLifecycleEventIds.get(sessionId)?.has(eventId)) {
        this.writeSessionEventNotification({
          schemaVersion: 2,
          sessionId: boundedString(sessionId, 256),
          conversationId,
          eventId,
          event: safeEvent,
        });
        this.markLifecycleEventPublished(sessionId, eventId);
      }
    });
  }

  private writeSessionEventNotification(params: Record<string, unknown>): void {
    if (Buffer.byteLength(JSON.stringify(params)) > 64 * 1024) {
      throw new Error("refused oversized Buzz session lifecycle event");
    }
    this.output.write(notification("_buzz/session/event", params));
  }

  private queueLifecycleTask(
    sessionId: string,
    operation: () => Promise<void>,
  ): Promise<void> {
    const previous = this.lifecycleTasks.get(sessionId) ?? Promise.resolve();
    const task = previous.catch(() => {}).then(operation);
    this.lifecycleTasks.set(sessionId, task);
    void task
      .catch((error: unknown) => {
        const failure =
          error instanceof Error ? error : new Error(String(error));
        // A persistence failure deliberately poisons the live ACP session. It
        // is unsafe to return a successful prompt after dropping a lifecycle
        // notice. `/new`, reset, or disposal creates the recovery boundary.
        if (!this.lifecycleFailures.has(sessionId)) {
          this.lifecycleFailures.set(sessionId, failure);
        }
        this.logger.error("failed to persist Buzz session lifecycle event", {
          sessionId,
          error: failure.message,
        });
      })
      .finally(() => {
        if (this.lifecycleTasks.get(sessionId) === task) {
          this.lifecycleTasks.delete(sessionId);
        }
      });
    return task;
  }

  private async flushLifecycleTasks(sessionId: string): Promise<void> {
    await this.settleLifecycleTasks(sessionId);
    const failure = this.lifecycleFailures.get(sessionId);
    if (failure) throw failure;
  }

  private async settleLifecycleTasks(sessionId: string): Promise<void> {
    for (;;) {
      const task = this.lifecycleTasks.get(sessionId);
      if (!task) return;
      await task.catch(() => {});
      if (this.lifecycleTasks.get(sessionId) === task) return;
    }
  }

  private markLifecycleEventPublished(
    sessionId: string,
    eventId: string,
  ): void {
    const published =
      this.publishedLifecycleEventIds.get(sessionId) ?? new Set<string>();
    published.add(eventId);
    this.publishedLifecycleEventIds.set(sessionId, published);
  }

  private clearLifecycleState(sessionId: string): void {
    this.lifecycleFailures.delete(sessionId);
    this.publishedLifecycleEventIds.delete(sessionId);
    if (!this.registry.hasSession(sessionId)) {
      this.lifecycleTasks.delete(sessionId);
    }
  }

  private pruneDisposedLifecycleState(): void {
    const sessionIds = new Set([
      ...this.lifecycleFailures.keys(),
      ...this.publishedLifecycleEventIds.keys(),
    ]);
    for (const sessionId of sessionIds) {
      if (!this.registry.hasSession(sessionId)) {
        this.clearLifecycleState(sessionId);
      }
    }
  }

  usageUpdate(
    sessionId: string,
    usage: SessionUsageSnapshot,
    contextLimit: number,
  ): void {
    this.output.write(
      notification("_goose/unstable/session/update", {
        sessionId,
        update: {
          sessionUpdate: "usage_update",
          ...(usage.contextTokens === null
            ? {}
            : { used: nonNegativeInteger(usage.contextTokens) }),
          contextLimit: nonNegativeInteger(contextLimit),
          accumulatedInputTokens: nonNegativeInteger(
            usage.accumulatedInputTokens,
          ),
          accumulatedOutputTokens: nonNegativeInteger(
            usage.accumulatedOutputTokens,
          ),
          accumulatedCachedInputTokens: Math.min(
            nonNegativeInteger(usage.accumulatedCachedInputTokens),
            nonNegativeInteger(usage.accumulatedInputTokens),
          ),
          ...(usage.accumulatedCost === null
            ? {}
            : { accumulatedCost: nonNegativeNumber(usage.accumulatedCost) }),
          ...(usage.model === null
            ? {}
            : { model: boundedString(usage.model, 256) }),
        },
      }),
    );
  }

  private async dispatch(message: JsonRpcInbound): Promise<void> {
    if (!("id" in message)) {
      await this.handleNotification(message.method, message.params);
      return;
    }
    try {
      const result = await this.handleRequest(message.method, message.params);
      this.output.write(response(message.id, result));
      if (message.method === "shutdown")
        queueMicrotask(() => void this.shutdown());
    } catch (error) {
      this.logger.warn("ACP request failed", {
        method: message.method,
        error: error instanceof Error ? error.message : String(error),
      });
      this.output.write(errorResponse(message.id, normalizeError(error)));
    }
  }

  private async handleRequest(
    method: string,
    rawParams: unknown,
  ): Promise<unknown> {
    if (method === "initialize") {
      if (this.initialized)
        throw new JsonRpcError(
          INVALID_PARAMS,
          "initialize may only be called once",
        );
      return this.initialize(rawParams);
    }
    if (this.closing)
      throw new JsonRpcError(INVALID_PARAMS, "agent is shutting down");
    if (!this.initialized)
      throw new JsonRpcError(INVALID_PARAMS, "initialize must be called first");
    this.pruneDisposedLifecycleState();

    switch (method) {
      case "session/new":
        return this.sessionNew(rawParams);
      case "session/prompt":
        return this.sessionPrompt(rawParams);
      case "session/set_model":
        return this.sessionSetModel(rawParams);
      case "session/set_config_option":
        return this.sessionSetConfig(rawParams);
      case "session/cancel":
        await this.sessionCancel(rawParams);
        return null;
      case "_session/steering":
        return this.sessionSteer(rawParams);
      case "_buzz/session/dispose":
        return this.sessionDispose(rawParams);
      case "_buzz/conversation/reset":
        return this.conversationReset(rawParams);
      case "_buzz/session/event_ack":
        return this.sessionEventAck(rawParams);
      case "shutdown":
        return { shutdown: true };
      default:
        throw new JsonRpcError(METHOD_NOT_FOUND, `Method not found: ${method}`);
    }
  }

  private async handleNotification(
    method: string,
    rawParams: unknown,
  ): Promise<void> {
    if (method === "session/cancel") {
      await this.sessionCancel(rawParams);
      return;
    }
    if (method === "exit") {
      await this.shutdown();
    }
  }

  private initialize(rawParams: unknown): Record<string, unknown> {
    const params = asRecord(rawParams ?? {}, "initialize params");
    const requested =
      typeof params.protocolVersion === "number" ? params.protocolVersion : 1;
    this.initialized = true;
    return {
      protocolVersion: Math.min(2, Math.max(1, Math.floor(requested))),
      agentCapabilities: {
        loadSession: false,
        promptCapabilities: {
          image: true,
          audio: false,
          embeddedContext: false,
        },
        mcpCapabilities: { http: false, sse: false },
      },
      agentInfo: { name: "buzz-pi-agent", version: "0.1.0" },
      _meta: {
        steering: { supported: true },
        buzz: {
          conversationPersistence: true,
          sessionEvents: {
            supported: true,
            durableReplay: true,
            ack: true,
            schemaVersion: 2,
          },
          contextLimitTokens: this.config.contextLimitTokens,
          sessionTranscriptMaxBytes: this.config.maxSessionFileBytes,
          threadSessions: {
            supported: true,
            persistence: true,
            dispose: true,
            resetToken: true,
            resetCommit: true,
          },
        },
      },
    };
  }

  private async sessionNew(
    rawParams: unknown,
  ): Promise<Record<string, unknown>> {
    const params = asRecord(rawParams, "session/new params");
    const cwd = requiredString(params, "cwd");
    if (!isAbsolute(cwd))
      throw new JsonRpcError(INVALID_PARAMS, "cwd must be absolute");
    const systemPrompt = optionalString(params.systemPrompt, "systemPrompt");
    if (
      systemPrompt &&
      Buffer.byteLength(systemPrompt) > MAX_SYSTEM_PROMPT_BYTES
    ) {
      throw new JsonRpcError(
        INVALID_PARAMS,
        `systemPrompt exceeds ${MAX_SYSTEM_PROMPT_BYTES} bytes`,
      );
    }
    const meta =
      params._meta === undefined ? undefined : asRecord(params._meta, "_meta");
    const buzzMeta =
      meta?.buzz === undefined ? undefined : asRecord(meta.buzz, "_meta.buzz");
    const conversationId = optionalString(
      buzzMeta?.conversationId,
      "conversationId",
    );
    const resetToken = optionalString(buzzMeta?.resetToken, "resetToken");
    const title = optionalString(meta?.sessionTitle, "sessionTitle");
    if (title && title.length > 256) {
      throw new JsonRpcError(
        INVALID_PARAMS,
        "sessionTitle must not exceed 256 characters",
      );
    }
    if (Array.isArray(params.mcpServers) && params.mcpServers.length > 0) {
      this.logger.warn(
        "session/new supplied MCP servers; Pi SDK does not consume ACP MCP config",
        {
          count: params.mcpServers.length,
        },
      );
    }
    const {
      sessionId,
      handle,
      resumedConversation = false,
      skipRelayHistory = false,
      lifecycleGeneration,
    } = await this.registry.create({
      cwd,
      ...(systemPrompt === undefined ? {} : { systemPrompt }),
      ...(title === undefined ? {} : { title }),
      ...(conversationId === undefined ? {} : { conversationId }),
      ...(resetToken === undefined ? {} : { resetToken }),
    });
    await handle.replayLifecycleEvents?.();
    await this.flushLifecycleTasks(sessionId);
    this.emitAvailableCommands(sessionId, handle.getResources().commands);
    const pendingReplay = conversationId
      ? await this.registry.listPendingSessionEvents(conversationId)
      : [];
    if (conversationId && lifecycleGeneration === undefined) {
      throw new Error("mapped Pi session has no lifecycle generation");
    }
    const replayable: typeof pendingReplay = [];
    for (const pending of pendingReplay) {
      if (pending.lifecycleGeneration !== lifecycleGeneration) {
        // Pi IDs can change during a continuity-preserving cwd/stale/lease
        // recovery. Only an authenticated reset rotates this durable epoch, so
        // mismatched records are specifically pre-reset and may be retired.
        await this.registry.acknowledgeSessionEvent(
          pending.conversationId,
          pending.eventId,
        );
        continue;
      }
      if (
        !this.publishedLifecycleEventIds.get(sessionId)?.has(pending.eventId)
      ) {
        replayable.push(pending);
      }
    }
    if (replayable.length > 0 && conversationId) {
      this.queueLifecycleTask(sessionId, async () => {
        // Defer until after dispatch writes the session/new response, so Buzz
        // learns the new outer sessionId before seeing replay notifications.
        await new Promise<void>((resolvePromise) => {
          setImmediate(resolvePromise);
        });
        if (
          !this.registry.hasSession(sessionId) ||
          this.registry.conversationIdForSession(sessionId) !== conversationId
        ) {
          return;
        }
        const activeHandle = await this.registry.get(sessionId);
        if (activeHandle.piSessionId !== handle.piSessionId) return;
        for (const pending of replayable) {
          this.writeSessionEventNotification({
            schemaVersion: 2,
            sessionId: boundedString(sessionId, 256),
            conversationId,
            eventId: pending.eventId,
            event: pending.event,
          });
          this.markLifecycleEventPublished(sessionId, pending.eventId);
        }
      });
    }
    return sessionDescription(
      sessionId,
      handle,
      this.config,
      resumedConversation,
      skipRelayHistory,
    );
  }

  private async sessionPrompt(
    rawParams: unknown,
  ): Promise<Record<string, unknown>> {
    const params = asRecord(rawParams, "session/prompt params");
    const sessionId = requiredString(params, "sessionId");
    const blocks = parsePromptBlocks(params.prompt);
    const text = blocks
      .filter(
        (block): block is Extract<AcpContentBlock, { type: "text" }> =>
          block.type === "text",
      )
      .map((block) => block.text)
      .join("\n\n");
    const images = blocks.filter(
      (block): block is AcpImageBlock => block.type === "image",
    );
    const session = await this.registry.get(sessionId);
    let stopReason: Awaited<ReturnType<AgentSessionHandle["prompt"]>>;
    try {
      stopReason = await session.prompt(text, images);
    } finally {
      await this.flushLifecycleTasks(sessionId);
    }
    return { stopReason };
  }

  private async sessionEventAck(
    rawParams: unknown,
  ): Promise<{ acknowledged: true }> {
    const params = asRecord(rawParams, "_buzz/session/event_ack params");
    const conversationId = requiredString(params, "conversationId");
    const eventId = requiredString(params, "eventId");
    if (conversationId.length > 512) {
      throw new JsonRpcError(
        INVALID_PARAMS,
        "conversationId must not exceed 512 characters",
      );
    }
    if (!LOWERCASE_UUID_PATTERN.test(eventId)) {
      throw new JsonRpcError(
        INVALID_PARAMS,
        "eventId must be a lowercase UUID",
      );
    }
    await this.registry.acknowledgeSessionEvent(conversationId, eventId);
    for (const [sessionId, published] of this.publishedLifecycleEventIds) {
      if (
        this.registry.conversationIdForSession(sessionId) === conversationId
      ) {
        published.delete(eventId);
        if (published.size === 0) {
          this.publishedLifecycleEventIds.delete(sessionId);
        }
      }
    }
    return { acknowledged: true };
  }

  private async sessionSetModel(
    rawParams: unknown,
  ): Promise<Record<string, unknown>> {
    const params = asRecord(rawParams, "session/set_model params");
    const session = await this.registry.get(
      requiredString(params, "sessionId"),
    );
    await session.setModel(requiredString(params, "modelId"));
    return { modelId: session.getContextSnapshot().model };
  }

  private async sessionSetConfig(
    rawParams: unknown,
  ): Promise<Record<string, unknown>> {
    const params = asRecord(rawParams, "session/set_config_option params");
    const session = await this.registry.get(
      requiredString(params, "sessionId"),
    );
    const configId = requiredString(params, "configId");
    const value = requiredString(params, "value");
    if (configId === "model") await session.setModel(value);
    else if (configId === "thinking") await session.setThinkingLevel(value);
    else
      throw new JsonRpcError(
        INVALID_PARAMS,
        `Unknown config option ${configId}`,
      );
    return { configOptions: sessionConfigOptions(session) };
  }

  private async sessionCancel(rawParams: unknown): Promise<void> {
    const params = asRecord(rawParams, "session/cancel params");
    const session = await this.registry.get(
      requiredString(params, "sessionId"),
    );
    await session.abort();
  }

  private async sessionSteer(
    rawParams: unknown,
  ): Promise<Record<string, unknown>> {
    const params = asRecord(rawParams, "_session/steering params");
    const session = await this.registry.get(
      requiredString(params, "sessionId"),
    );
    const blocks = parsePromptBlocks(params.prompt);
    const text = blocks
      .filter((block) => block.type === "text")
      .map((block) => block.text)
      .join("\n\n");
    const wasBusy = session.isBusy;
    await session.steer(text);
    return { outcome: wasBusy ? "injected" : "startedNewTurn" };
  }

  private async sessionDispose(
    rawParams: unknown,
  ): Promise<Record<string, unknown>> {
    const params = asRecord(rawParams, "_buzz/session/dispose params");
    const sessionId = requiredString(params, "sessionId");
    const forget = params.forget === true;
    await this.settleLifecycleTasks(sessionId);
    try {
      const disposed = await this.registry.disposeSession(sessionId, forget);
      return { disposed, forgotten: disposed && forget };
    } finally {
      if (!this.registry.hasSession(sessionId)) {
        this.clearLifecycleState(sessionId);
      }
    }
  }

  private async conversationReset(
    rawParams: unknown,
  ): Promise<Record<string, unknown>> {
    const params = asRecord(rawParams, "_buzz/conversation/reset params");
    const conversationId = boundedRequiredString(
      params.conversationId,
      "conversationId",
      512,
    );
    const resetToken = boundedRequiredString(
      params.resetToken,
      "resetToken",
      512,
    );
    const affectedSessionIds = new Set(
      [
        ...this.lifecycleTasks.keys(),
        ...this.lifecycleFailures.keys(),
        ...this.publishedLifecycleEventIds.keys(),
      ].filter(
        (sessionId) =>
          this.registry.conversationIdForSession(sessionId) === conversationId,
      ),
    );
    await Promise.allSettled(
      [...affectedSessionIds].map((sessionId) =>
        this.settleLifecycleTasks(sessionId),
      ),
    );
    try {
      return await this.registry.commitConversationReset(
        conversationId,
        resetToken,
      );
    } finally {
      for (const sessionId of affectedSessionIds) {
        if (!this.registry.hasSession(sessionId)) {
          this.clearLifecycleState(sessionId);
        }
      }
    }
  }

  private emitAvailableCommands(
    sessionId: string,
    commands: ResourceSnapshot["commands"],
  ): void {
    this.sessionUpdate(sessionId, {
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
}

function sessionDescription(
  sessionId: string,
  handle: AgentSessionHandle,
  config: AdapterConfig,
  resumedConversation: boolean,
  skipRelayHistory: boolean,
): Record<string, unknown> {
  const models = handle.getModels();
  const currentModelId = handle.getContextSnapshot().model;
  return {
    sessionId,
    configOptions: sessionConfigOptions(handle),
    models: {
      currentModelId,
      availableModels: models.map((model) => ({
        modelId: model.id,
        name: model.name,
        ...(model.description === undefined
          ? {}
          : { description: model.description }),
      })),
    },
    _meta: {
      buzz: {
        piSessionId: handle.piSessionId,
        persistent: handle.sessionFile !== undefined,
        resumedConversation,
        skipRelayHistory,
        contextLimitTokens: config.contextLimitTokens,
        effectiveLimitTokens: handle.getContextSnapshot().effectiveLimitTokens,
        compactionThresholdTokens:
          handle.getContextSnapshot().compactionThresholdTokens,
      },
    },
  };
}

function sessionConfigOptions(
  handle: AgentSessionHandle,
): Record<string, unknown>[] {
  const snapshot = handle.getContextSnapshot();
  return [
    {
      configId: "model",
      category: "model",
      displayName: "Model",
      value: snapshot.model,
      options: handle.getModels().map((model) => ({
        value: model.id,
        displayName: model.name,
      })),
    },
    {
      configId: "thinking",
      category: "thought",
      displayName: "Thinking",
      value: snapshot.thinkingLevel,
      options: handle
        .getThinkingLevels()
        .map((level) => ({ value: level, displayName: level })),
    },
  ];
}

function parsePromptBlocks(value: unknown): AcpContentBlock[] {
  if (!Array.isArray(value) || value.length === 0) {
    throw new JsonRpcError(INVALID_PARAMS, "prompt must be a non-empty array");
  }
  if (value.length > MAX_PROMPT_BLOCKS) {
    throw new JsonRpcError(
      INVALID_PARAMS,
      `prompt exceeds ${MAX_PROMPT_BLOCKS} blocks`,
    );
  }
  const blocks = value.map((item, index): AcpContentBlock => {
    const block = asRecord(item, `prompt[${index}]`);
    if (block.type === "text" && typeof block.text === "string") {
      return { type: "text", text: block.text };
    }
    if (
      block.type === "image" &&
      typeof block.data === "string" &&
      typeof block.mimeType === "string"
    ) {
      return { type: "image", data: block.data, mimeType: block.mimeType };
    }
    throw new JsonRpcError(
      INVALID_PARAMS,
      `Unsupported prompt block at index ${index}`,
    );
  });
  const size = Buffer.byteLength(JSON.stringify(blocks));
  if (size > MAX_PROMPT_BYTES) {
    throw new JsonRpcError(
      INVALID_PARAMS,
      `prompt exceeds ${MAX_PROMPT_BYTES} bytes`,
    );
  }
  return blocks;
}

function optionalString(value: unknown, name: string): string | undefined {
  if (value === undefined || value === null) return undefined;
  if (typeof value !== "string" || value.trim() === "") {
    throw new JsonRpcError(
      INVALID_PARAMS,
      `${name} must be a non-empty string`,
    );
  }
  return value;
}

function boundedRequiredString(
  value: unknown,
  name: string,
  maxLength: number,
): string {
  const result = requiredString({ [name]: value }, name);
  if (result.length > maxLength) {
    throw new JsonRpcError(
      INVALID_PARAMS,
      `${name} must not exceed ${maxLength} characters`,
    );
  }
  return result;
}

function normalizeError(error: unknown): JsonRpcError {
  if (error instanceof JsonRpcError) return error;
  const message = error instanceof Error ? error.message : String(error);
  if (message.startsWith("BUZZ_CONTEXT_LIMIT:")) {
    return new JsonRpcError(
      AGENT_CONTEXT_LIMIT,
      boundedString(message, 2_000),
      {
        kind: "context_limit",
        retryable: false,
      },
    );
  }
  if (message.startsWith("BUZZ_SESSION_STORAGE_LIMIT:")) {
    return new JsonRpcError(
      AGENT_SESSION_STORAGE_LIMIT,
      boundedString(message, 2_000),
      {
        kind: "session_storage_limit",
        retryable: false,
        recovery: "/new",
      },
    );
  }
  if (message.startsWith("BUZZ_PI_SESSION_INVALIDATED:")) {
    return new JsonRpcError(
      AGENT_SESSION_INVALIDATED,
      boundedString(message, 2_000),
      {
        kind: "session_invalidated",
        retryable: true,
      },
    );
  }
  if (
    message.includes("Unknown") ||
    message.includes("must be") ||
    message.includes("already")
  ) {
    return new JsonRpcError(INVALID_PARAMS, boundedString(message, 2_000));
  }
  return new JsonRpcError(INTERNAL_ERROR, boundedString(message, 2_000));
}

interface SanitizeLimits {
  maxDepth: number;
  maxArrayItems: number;
  maxObjectEntries: number;
  maxStringCharacters: number;
  maxKeyCharacters: number;
  maxNodes: number;
  maxBytes: number;
}

interface SanitizeBudget {
  remainingNodes: number;
  remainingBytes: number;
  seen: WeakSet<object>;
}

function sanitizeValue(value: unknown): unknown {
  return sanitizeWithBudget(value, {
    maxDepth: 8,
    maxArrayItems: 50,
    maxObjectEntries: 100,
    maxStringCharacters: 8_192,
    maxKeyCharacters: 128,
    maxNodes: 2_048,
    maxBytes: 256 * 1_024,
  });
}

function sanitizeLifecycleValue(value: unknown): unknown {
  return sanitizeWithBudget(value, {
    maxDepth: 6,
    maxArrayItems: 20,
    maxObjectEntries: 40,
    maxStringCharacters: 1_024,
    maxKeyCharacters: 64,
    maxNodes: 512,
    maxBytes: 48 * 1_024,
  });
}

function sanitizeWithBudget(value: unknown, limits: SanitizeLimits): unknown {
  return sanitizeNode(value, 0, limits, {
    remainingNodes: limits.maxNodes,
    remainingBytes: limits.maxBytes,
    seen: new WeakSet(),
  });
}

function sanitizeNode(
  value: unknown,
  depth: number,
  limits: SanitizeLimits,
  budget: SanitizeBudget,
): unknown {
  if (
    depth > limits.maxDepth ||
    budget.remainingNodes <= 0 ||
    budget.remainingBytes <= 0
  ) {
    return "[truncated]";
  }
  budget.remainingNodes -= 1;

  if (typeof value === "string") {
    return takeBudgetedString(value, limits.maxStringCharacters, budget);
  }
  if (typeof value === "number") {
    budget.remainingBytes = Math.max(0, budget.remainingBytes - 16);
    return Number.isFinite(value) ? value : 0;
  }
  if (typeof value === "boolean" || value === null || value === undefined) {
    budget.remainingBytes = Math.max(0, budget.remainingBytes - 8);
    return value;
  }
  if (typeof value !== "object") {
    return takeBudgetedString(
      String(value),
      limits.maxStringCharacters,
      budget,
    );
  }
  if (budget.seen.has(value)) {
    return takeBudgetedString("[circular]", 16, budget);
  }
  budget.seen.add(value);
  budget.remainingBytes = Math.max(0, budget.remainingBytes - 2);

  if (Array.isArray(value)) {
    const result: unknown[] = [];
    const count = Math.min(value.length, limits.maxArrayItems);
    for (let index = 0; index < count; index += 1) {
      if (budget.remainingNodes <= 0 || budget.remainingBytes <= 0) {
        result.push("[truncated]");
        break;
      }
      result.push(sanitizeNode(value[index], depth + 1, limits, budget));
    }
    return result;
  }

  const result: Record<string, unknown> = {};
  let entries = 0;
  for (const key in value) {
    if (!Object.hasOwn(value, key)) continue;
    if (entries >= limits.maxObjectEntries) break;
    if (budget.remainingNodes <= 0 || budget.remainingBytes <= 0) {
      result["[truncated]"] = true;
      break;
    }
    const safeKey = takeBudgetedString(key, limits.maxKeyCharacters, budget);
    let item: unknown;
    try {
      item = (value as Record<string, unknown>)[key];
    } catch {
      item = "[unavailable]";
    }
    result[safeKey] = sanitizeNode(item, depth + 1, limits, budget);
    entries += 1;
  }
  return result;
}

function takeBudgetedString(
  value: string,
  maxCharacters: number,
  budget: SanitizeBudget,
): string {
  const allowedCharacters = Math.max(
    1,
    Math.min(maxCharacters, Math.floor(budget.remainingBytes / 4)),
  );
  const truncated = value.length > allowedCharacters;
  const result = truncated
    ? `${value.slice(0, Math.max(0, allowedCharacters - 1))}…`
    : value;
  budget.remainingBytes = Math.max(
    0,
    budget.remainingBytes - Buffer.byteLength(result),
  );
  return result;
}

function nonNegativeInteger(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(Number.MAX_SAFE_INTEGER, Math.round(value)));
}

function nonNegativeNumber(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, value);
}

function boundedString(value: string, maximum: number): string {
  return value.length <= maximum ? value : `${value.slice(0, maximum)}…`;
}

function isControlMethod(method: string): boolean {
  return [
    "session/cancel",
    "_session/steering",
    "_buzz/session/dispose",
    "_buzz/conversation/reset",
    "_buzz/session/event_ack",
    "shutdown",
    "exit",
  ].includes(method);
}
