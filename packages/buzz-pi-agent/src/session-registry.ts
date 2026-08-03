import { randomUUID } from "node:crypto";
import type { AdapterConfig } from "./config.js";
import type {
  ConversationStore,
  ResolveConversationResult,
} from "./conversation-store.js";
import { INVALID_PARAMS, JsonRpcError } from "./wire.js";
import type {
  AdapterEventSink,
  AgentSessionFactory,
  AgentSessionHandle,
  BuzzSessionEvent,
  Logger,
  PendingBuzzSessionEvent,
} from "./types.js";
import { captureWorkspaceIdentity } from "./workspace.js";

interface LiveSession {
  handle: AgentSessionHandle;
  lastUsedMs: number;
  conversationId?: string;
  resetToken?: string;
  lifecycleGeneration?: string;
  closing?: boolean;
  disposeForget?: boolean;
  disposePromise?: Promise<boolean>;
  refreshConversation?: () => Promise<boolean>;
  forgetConversation?: () => Promise<string | undefined>;
  releaseConversation?: () => Promise<void>;
}

export interface ConversationLifecycleIdentity {
  conversationId: string;
  lifecycleGeneration: string;
  readiness?: Promise<void>;
  deferPublication?: boolean;
}

interface PendingLifecycleRoute {
  identity: ConversationLifecycleIdentity;
  resolve: () => void;
  reject: (error: Error) => void;
}

export class SessionRegistry {
  private readonly live = new Map<string, LiveSession>();
  private readonly conversationToSession = new Map<string, string>();
  private readonly pendingLifecycleRoutes = new Map<
    string,
    PendingLifecycleRoute
  >();
  private readonly pendingConversations = new Map<
    string,
    {
      resetToken: string | undefined;
      cwd: string;
      promise: Promise<{
        sessionId: string;
        handle: AgentSessionHandle;
        resumedConversation: boolean;
        skipRelayHistory: boolean;
        lifecycleGeneration?: string;
      }>;
    }
  >();
  private readonly pendingResets = new Map<
    string,
    {
      resetToken: string;
      promise: Promise<{ committed: true; alreadyCommitted: boolean }>;
    }
  >();
  private sweepTimer: NodeJS.Timeout | undefined;
  private closing = false;
  private capacityMutex: Promise<void> = Promise.resolve();
  private pendingCapacity = 0;

  constructor(
    private readonly factory: AgentSessionFactory,
    private readonly conversations: ConversationStore,
    private readonly config: AdapterConfig,
    private readonly eventSink: AdapterEventSink,
    private readonly logger: Logger,
    private readonly now: () => number = Date.now,
  ) {
    this.factory.setInvalidationHandler?.((sessionIds, error) =>
      this.invalidateSessions(sessionIds, error),
    );
  }

  async start(): Promise<void> {
    await this.conversations.initialize();
    this.sweepTimer = setInterval(() => {
      void this.sweepExpired();
    }, this.config.sweepIntervalMs);
    this.sweepTimer.unref();
  }

  async create(options: {
    cwd: string;
    systemPrompt?: string;
    title?: string;
    conversationId?: string;
    resetToken?: string;
  }): Promise<{
    sessionId: string;
    handle: AgentSessionHandle;
    resumedConversation: boolean;
    skipRelayHistory: boolean;
    lifecycleGeneration?: string;
  }> {
    this.assertOpen();
    const workspace = captureWorkspaceIdentity(options.cwd);
    const normalizedOptions = {
      ...options,
      cwd: workspace.canonicalPath,
      requestedCwd: workspace.requestedPath,
    };
    if (normalizedOptions.conversationId) {
      const pendingReset = this.pendingResets.get(
        normalizedOptions.conversationId,
      );
      if (pendingReset) await pendingReset.promise;
      this.assertOpen();
      const existingSessionId = this.conversationToSession.get(
        normalizedOptions.conversationId,
      );
      if (existingSessionId) {
        const existing = this.live.get(existingSessionId);
        if (existing?.closing) {
          if (!existing.disposePromise) {
            throw new Error("Pi session is closing without a disposal fence");
          }
          await existing.disposePromise;
          this.assertOpen();
        } else if (existing?.handle.isValid === false) {
          await this.disposeSession(existingSessionId, false);
        } else if (existing?.handle.cwd !== normalizedOptions.cwd) {
          await this.disposeSession(existingSessionId, false);
        } else if (
          !normalizedOptions.resetToken ||
          normalizedOptions.resetToken === existing?.resetToken
        ) {
          return {
            sessionId: existingSessionId,
            handle: await this.get(existingSessionId),
            resumedConversation: true,
            skipRelayHistory: false,
            ...(existing?.lifecycleGeneration === undefined
              ? {}
              : { lifecycleGeneration: existing.lifecycleGeneration }),
          };
        } else await this.disposeSession(existingSessionId, false);
      }
      const pending = this.pendingConversations.get(
        normalizedOptions.conversationId,
      );
      if (pending) {
        if (pending.cwd !== normalizedOptions.cwd) {
          throw new JsonRpcError(
            INVALID_PARAMS,
            "A different workspace is already creating this conversation",
          );
        }
        if (pending.resetToken !== normalizedOptions.resetToken) {
          throw new JsonRpcError(
            INVALID_PARAMS,
            "A different reset token is already creating this conversation",
          );
        }
        return pending.promise;
      }
      const creation = this.createConversationSession({
        ...normalizedOptions,
        conversationId: normalizedOptions.conversationId,
      }).finally(() => {
        this.pendingConversations.delete(
          normalizedOptions.conversationId as string,
        );
      });
      this.pendingConversations.set(normalizedOptions.conversationId, {
        resetToken: normalizedOptions.resetToken,
        cwd: normalizedOptions.cwd,
        promise: creation,
      });
      return creation;
    }
    return this.createUnmappedSession(normalizedOptions);
  }

  async commitConversationReset(
    conversationId: string,
    resetToken: string,
  ): Promise<{ committed: true; alreadyCommitted: boolean }> {
    this.assertOpen();
    const pendingCreation = this.pendingConversations.get(conversationId);
    if (pendingCreation) await pendingCreation.promise;
    this.assertOpen();
    const pending = this.pendingResets.get(conversationId);
    if (pending) {
      if (pending.resetToken !== resetToken) {
        throw new JsonRpcError(
          INVALID_PARAMS,
          "A different reset token is already committing this conversation",
        );
      }
      return pending.promise;
    }
    const commit = this.commitConversationResetInner(
      conversationId,
      resetToken,
    ).finally(() => {
      this.pendingResets.delete(conversationId);
    });
    this.pendingResets.set(conversationId, { resetToken, promise: commit });
    return commit;
  }

  async get(sessionId: string): Promise<AgentSessionHandle> {
    this.assertOpen();
    const session = this.live.get(sessionId);
    if (!session)
      throw new JsonRpcError(
        INVALID_PARAMS,
        `Unknown or disposed session ${sessionId}`,
      );
    if (session.closing) {
      throw new JsonRpcError(INVALID_PARAMS, `Session ${sessionId} is closing`);
    }
    if (session.handle.isValid === false) {
      throw new JsonRpcError(
        INVALID_PARAMS,
        `Invalidated runtime session ${sessionId}`,
      );
    }
    if (session.refreshConversation) {
      let refreshed = false;
      try {
        refreshed = await session.refreshConversation();
      } catch (error) {
        await this.disposeSession(sessionId, false).catch(() => {});
        throw error;
      }
      if (!refreshed) {
        await this.disposeSession(sessionId, false).catch(() => {});
        throw new JsonRpcError(
          INVALID_PARAMS,
          `Pi conversation lease was superseded for session ${sessionId}`,
        );
      }
    }
    session.lastUsedMs = this.now();
    return session.handle;
  }

  async disposeSession(sessionId: string, forget = false): Promise<boolean> {
    const session = this.live.get(sessionId);
    if (!session) return false;
    if (session.closing) {
      if (forget && session.disposeForget !== true) {
        throw new Error(
          `Session ${sessionId} is already closing without a forget boundary`,
        );
      }
      if (!session.disposePromise) {
        throw new Error("Pi session is closing without a disposal fence");
      }
      return session.disposePromise;
    }
    session.closing = true;
    session.disposeForget = forget;
    const disposePromise = this.disposeSessionInner(session).finally(() => {
      if (this.live.get(sessionId) === session) this.live.delete(sessionId);
      if (
        session.conversationId &&
        this.conversationToSession.get(session.conversationId) === sessionId
      ) {
        this.conversationToSession.delete(session.conversationId);
      }
    });
    session.disposePromise = disposePromise;
    return disposePromise;
  }

  private async disposeSessionInner(session: LiveSession): Promise<boolean> {
    const forget = session.disposeForget === true;
    const failures: unknown[] = [];
    let forgottenFile: string | undefined;
    if (forget && session.conversationId) {
      // Remove the durable route first: a concurrent session/new must never
      // reopen the context being reset.
      try {
        if (!session.forgetConversation) {
          throw new Error("Pi conversation session has no forget fence");
        }
        forgottenFile = await session.forgetConversation();
      } catch (error) {
        failures.push(error);
      }
    }
    try {
      if (session.handle.isBusy) {
        try {
          await session.handle.abort();
        } catch (error) {
          failures.push(error);
        }
      }
      try {
        await session.handle.dispose();
      } catch (error) {
        failures.push(error);
      }
      // A mapped conversation file is only ours to delete when the
      // conditional manifest removal succeeded. An unmapped session has no
      // durable route and can never be resumed, so every graceful disposal
      // must remove its otherwise-inaccessible transcript.
      const fileToDelete = session.conversationId
        ? forgottenFile
        : session.handle.sessionFile;
      if (fileToDelete) {
        try {
          await this.conversations.deleteSessionFile(fileToDelete);
        } catch (error) {
          failures.push(error);
        }
      }
    } finally {
      // Release even after a failed forget. Otherwise the still-present
      // mapping retains a live-PID lease and /new cannot recover until restart.
      try {
        await session.releaseConversation?.();
      } catch (error) {
        failures.push(error);
      }
    }
    if (failures.length === 1) throw failures[0];
    if (failures.length > 1) {
      throw new AggregateError(
        failures,
        "Pi session disposal encountered multiple failures",
      );
    }
    return true;
  }

  async sweepExpired(): Promise<number> {
    const cutoff = this.now() - this.config.sessionTtlMs;
    const expired = [...this.live.entries()]
      .filter(
        ([, session]) =>
          !session.closing &&
          !session.handle.isBusy &&
          session.lastUsedMs <= cutoff,
      )
      .map(([sessionId]) => sessionId);
    await Promise.all(expired.map((sessionId) => this.evict(sessionId, "ttl")));
    for (const [sessionId, session] of [...this.live.entries()]) {
      if (session.closing || !session.refreshConversation) continue;
      try {
        if (!(await session.refreshConversation())) {
          await this.disposeSession(sessionId, false);
        }
      } catch (error) {
        this.logger.warn("failed to refresh Pi conversation lease", {
          conversationId: session.conversationId,
          error: errorMessage(error),
        });
        // Treat an indeterminate refresh exactly like a rejected generation.
        // Continuing to serve the handle would let it append to a JSONL after
        // another adapter may have taken over the Buzz conversation.
        await this.disposeSession(sessionId, false).catch(
          (disposeError: unknown) => {
            this.logger.warn(
              "failed to dispose Pi session after lease refresh failure",
              {
                conversationId: session.conversationId,
                error: errorMessage(disposeError),
              },
            );
          },
        );
      }
    }
    const active = new Set(
      [...this.live.values()]
        .map((session) => session.conversationId)
        .filter((value): value is string => value !== undefined),
    );
    await this.conversations.prune(active);
    return expired.length;
  }

  async shutdown(): Promise<void> {
    if (this.closing) return;
    this.closing = true;
    if (this.sweepTimer) clearInterval(this.sweepTimer);
    await Promise.allSettled(
      [...this.pendingConversations.values()].map((pending) => pending.promise),
    );
    await Promise.allSettled(
      [...this.pendingResets.values()].map((pending) => pending.promise),
    );
    const sessions = [...this.live.entries()];
    await Promise.allSettled(
      sessions.map(async ([sessionId]) => {
        try {
          await this.disposeSession(sessionId, false);
        } catch (error) {
          this.logger.warn("failed to dispose session during shutdown", {
            sessionId,
            error: errorMessage(error),
          });
        }
      }),
    );
    this.live.clear();
    this.conversationToSession.clear();
    this.pendingLifecycleRoutes.clear();
  }

  get size(): number {
    return this.live.size;
  }

  hasSession(sessionId: string): boolean {
    const session = this.live.get(sessionId);
    return session !== undefined && session.closing !== true;
  }

  conversationIdForSession(sessionId: string): string | undefined {
    return this.live.get(sessionId)?.conversationId;
  }

  conversationIdentityForSession(
    sessionId: string,
  ): ConversationLifecycleIdentity | undefined {
    const pending = this.pendingLifecycleRoutes.get(sessionId);
    if (pending) return pending.identity;
    const session = this.live.get(sessionId);
    if (!session?.conversationId || !session.lifecycleGeneration) {
      return undefined;
    }
    return {
      conversationId: session.conversationId,
      lifecycleGeneration: session.lifecycleGeneration,
    };
  }

  async persistConversationSessionEvent(
    conversationId: string,
    lifecycleGeneration: string,
    eventId: string,
    event: BuzzSessionEvent,
  ): Promise<boolean> {
    return this.conversations.enqueueSessionEvent(
      conversationId,
      eventId,
      event,
      lifecycleGeneration,
    );
  }

  listPendingSessionEvents(
    conversationId: string,
  ): Promise<PendingBuzzSessionEvent[]> {
    return this.conversations.listPendingSessionEvents(conversationId);
  }

  acknowledgeSessionEvent(
    conversationId: string,
    eventId: string,
  ): Promise<void> {
    return this.conversations.acknowledgeSessionEvent(conversationId, eventId);
  }

  private async createConversationSession(options: {
    cwd: string;
    requestedCwd: string;
    systemPrompt?: string;
    title?: string;
    conversationId: string;
    resetToken?: string;
  }): Promise<{
    sessionId: string;
    handle: AgentSessionHandle;
    resumedConversation: boolean;
    skipRelayHistory: boolean;
    lifecycleGeneration: string;
  }> {
    const releaseCapacity = await this.reserveCapacity();
    try {
      const cwd = options.cwd;
      const sessionId = `ses_${randomUUID().replaceAll("-", "")}`;
      let handle: AgentSessionHandle | undefined;
      let openedPersisted = false;
      let pendingLifecycleRoute: PendingLifecycleRoute | undefined;
      let resolved: ResolveConversationResult;
      try {
        resolved = await this.conversations.resolve(
          options.conversationId,
          options.resetToken,
          cwd,
          async (persistedSessionFile, lifecycleGeneration) => {
            pendingLifecycleRoute = this.installPendingLifecycleRoute(
              sessionId,
              options.conversationId,
              lifecycleGeneration,
            );
            openedPersisted = persistedSessionFile !== undefined;
            handle = await this.factory.create({
              cwd,
              requestedCwd: options.requestedCwd,
              ...(options.systemPrompt === undefined
                ? {}
                : { systemPrompt: options.systemPrompt }),
              ...(options.title === undefined ? {} : { title: options.title }),
              ...(persistedSessionFile === undefined
                ? {}
                : { persistedSessionFile }),
              eventSink: this.eventSink,
              acpSessionId: sessionId,
            });
            if (!handle.sessionFile) {
              await handle.dispose();
              throw new Error(
                "Pi returned a non-persistent session for a persisted Buzz conversation",
              );
            }
            return {
              sessionFile: handle.sessionFile,
              piSessionId: handle.piSessionId,
              cwd: handle.cwd,
            };
          },
        );
      } catch (error) {
        pendingLifecycleRoute?.reject(asError(error));
        if (handle) {
          const file = handle.sessionFile;
          await handle.dispose().catch(() => {});
          if (!openedPersisted && file)
            await this.conversations.deleteSessionFile(file);
        }
        this.pendingLifecycleRoutes.delete(sessionId);
        throw error;
      }
      if (!handle)
        throw new Error(
          "Pi conversation session creation failed without a handle",
        );
      pendingLifecycleRoute?.resolve();
      // Finish cleanup while this creation is represented only by its reserved
      // slot. Exposing it in `live` before this await would count one handle as
      // both live and pending, allowing a concurrent capacity check to evict it
      // before this session/new returns.
      if (resolved.retiredSessionFile) {
        await this.conversations
          .deleteSessionFile(resolved.retiredSessionFile)
          .catch((error: unknown) => {
            // The manifest already points to the replacement generation. An
            // orphaned inactive JSONL is safer than failing a successfully
            // committed session transition and losing its live handle.
            this.logger.warn("failed to delete retired Pi session file", {
              conversationId: options.conversationId,
              error: errorMessage(error),
            });
          });
      }
      // Promotion is synchronous: no other capacity reservation can observe a
      // gap or double-count between releasing the pending slot and installing
      // the live handle.
      releaseCapacity();
      this.live.set(sessionId, {
        handle,
        lastUsedMs: this.now(),
        conversationId: options.conversationId,
        ...(options.resetToken === undefined
          ? {}
          : { resetToken: options.resetToken }),
        lifecycleGeneration: resolved.lifecycleGeneration,
        refreshConversation: resolved.refresh,
        forgetConversation: resolved.forget,
        releaseConversation: resolved.release,
      });
      this.pendingLifecycleRoutes.delete(sessionId);
      this.conversationToSession.set(options.conversationId, sessionId);
      if (resolved.previousPiSessionId) {
        const context = handle.getContextSnapshot();
        this.eventSink.buzzSessionEvent(sessionId, {
          type: "session_reset",
          timestamp: new Date().toISOString(),
          message: "Started a fresh Pi session for this Buzz thread.",
          piSessionId: handle.piSessionId,
          previousPiSessionId: resolved.previousPiSessionId,
          limitTokens: context.limitTokens,
          effectiveLimitTokens: context.effectiveLimitTokens,
          compactionThresholdTokens: context.compactionThresholdTokens,
        });
      }
      return {
        sessionId,
        handle,
        resumedConversation: resolved.resumed,
        skipRelayHistory: resolved.skipRelayHistory,
        lifecycleGeneration: resolved.lifecycleGeneration,
      };
    } finally {
      releaseCapacity();
    }
  }

  private async commitConversationResetInner(
    conversationId: string,
    resetToken: string,
  ): Promise<{ committed: true; alreadyCommitted: boolean }> {
    const result = await this.conversations.commitReset(
      conversationId,
      resetToken,
    );
    const liveSessionId = this.conversationToSession.get(conversationId);
    if (result.disposeLiveSession && liveSessionId !== undefined) {
      await this.disposeSession(liveSessionId, false);
    }
    if (result.retiredSessionFile !== undefined) {
      await this.conversations
        .deleteSessionFile(result.retiredSessionFile)
        .catch((error: unknown) => {
          // The durable route and tombstone are already committed. An orphaned
          // inactive JSONL cannot be reopened and is safer than turning a
          // successful reset into a retry/ACK ambiguity.
          this.logger.warn("failed to delete reset Pi session file", {
            conversationId,
            error: errorMessage(error),
          });
        });
    }
    return { committed: true, alreadyCommitted: result.alreadyCommitted };
  }

  private async createUnmappedSession(options: {
    cwd: string;
    requestedCwd: string;
    systemPrompt?: string;
    title?: string;
  }): Promise<{
    sessionId: string;
    handle: AgentSessionHandle;
    resumedConversation: boolean;
    skipRelayHistory: boolean;
    lifecycleGeneration?: string;
  }> {
    const releaseCapacity = await this.reserveCapacity();
    try {
      const sessionId = `ses_${randomUUID().replaceAll("-", "")}`;
      const handle = await this.factory.create({
        cwd: options.cwd,
        requestedCwd: options.requestedCwd,
        ...(options.systemPrompt === undefined
          ? {}
          : { systemPrompt: options.systemPrompt }),
        ...(options.title === undefined ? {} : { title: options.title }),
        eventSink: this.eventSink,
        acpSessionId: sessionId,
      });
      releaseCapacity();
      this.live.set(sessionId, { handle, lastUsedMs: this.now() });
      return {
        sessionId,
        handle,
        resumedConversation: false,
        skipRelayHistory: false,
      };
    } finally {
      releaseCapacity();
    }
  }

  private installPendingLifecycleRoute(
    sessionId: string,
    conversationId: string,
    lifecycleGeneration: string,
  ): PendingLifecycleRoute {
    const existing = this.pendingLifecycleRoutes.get(sessionId);
    if (existing) {
      if (
        existing.identity.conversationId !== conversationId ||
        existing.identity.lifecycleGeneration !== lifecycleGeneration
      ) {
        throw new Error("Pi creation lifecycle identity changed mid-session");
      }
      return existing;
    }
    let resolveReadiness: () => void = () => {};
    let rejectReadiness: (error: Error) => void = () => {};
    let settled = false;
    const readiness = new Promise<void>((resolvePromise, rejectPromise) => {
      resolveReadiness = resolvePromise;
      rejectReadiness = rejectPromise;
    });
    // A failed create can have no early event waiter. Mark the rejection
    // observed while preserving it for every actual waiter.
    void readiness.catch(() => {});
    const route: PendingLifecycleRoute = {
      identity: Object.freeze({
        conversationId,
        lifecycleGeneration,
        readiness,
        deferPublication: true,
      }),
      resolve: () => {
        if (settled) return;
        settled = true;
        resolveReadiness();
      },
      reject: (error) => {
        if (settled) return;
        settled = true;
        rejectReadiness(error);
      },
    };
    this.pendingLifecycleRoutes.set(sessionId, route);
    return route;
  }

  private async reserveCapacity(): Promise<() => void> {
    let unlock = (): void => {};
    const prior = this.capacityMutex;
    this.capacityMutex = new Promise<void>((resolveMutex) => {
      unlock = resolveMutex;
    });
    await prior;
    try {
      if (this.live.size + this.pendingCapacity >= this.config.maxSessions) {
        const candidate = [...this.live.entries()]
          .filter(([, session]) => !session.closing && !session.handle.isBusy)
          .sort((left, right) => left[1].lastUsedMs - right[1].lastUsedMs)[0];
        if (!candidate) {
          throw new JsonRpcError(
            INVALID_PARAMS,
            "Session capacity reached; all slots are busy or initializing",
          );
        }
        await this.evict(candidate[0], "lru");
      }
      this.pendingCapacity++;
      let released = false;
      return () => {
        if (released) return;
        released = true;
        this.pendingCapacity = Math.max(0, this.pendingCapacity - 1);
      };
    } finally {
      unlock();
    }
  }

  private async evict(sessionId: string, reason: "lru" | "ttl"): Promise<void> {
    const session = this.live.get(sessionId);
    if (!session || session.handle.isBusy) return;
    await this.disposeSession(sessionId, false);
    this.logger.info("evicted inactive Pi session", { sessionId, reason });
  }

  private assertOpen(): void {
    if (this.closing)
      throw new JsonRpcError(INVALID_PARAMS, "Agent is shutting down");
  }

  private async invalidateSessions(
    sessionIds: readonly string[],
    error: Error,
  ): Promise<void> {
    for (const sessionId of sessionIds) {
      // Use the same disposal invariant as explicit/LRU/TTL/shutdown paths:
      // retain mapped conversation transcripts, but remove an unmapped file
      // that has no durable route after this failed worker generation exits.
      await this.disposeSession(sessionId, false).catch(
        (disposeError: unknown) => {
          this.logger.warn(
            "failed to dispose session after runtime host failure",
            {
              sessionId,
              error: errorMessage(disposeError),
            },
          );
        },
      );
    }
    this.logger.error("invalidated Pi sessions after runtime host failure", {
      count: sessionIds.length,
      error: errorMessage(error),
    });
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function asError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}
