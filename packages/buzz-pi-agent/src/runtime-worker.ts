import { fork, type ChildProcess } from "node:child_process";
import { fileURLToPath } from "node:url";
import type { AdapterConfig } from "./config.js";
import { BoundedIpcSendQueue } from "./ipc-send-queue.js";
import type {
  CreateRuntimeResult,
  RuntimeHostEvent,
  RuntimeHostMessage,
  RuntimeHostRequest,
  RuntimeHostResponse,
  RuntimeSessionState,
} from "./runtime-host-protocol.js";
import type {
  AcpImageBlock,
  AdapterEventSink,
  AgentSessionFactory,
  AgentSessionHandle,
  ContextSnapshot,
  CreateSessionOptions,
  Logger,
  ModelDescriptor,
  ResourceSnapshot,
  SessionUsageSnapshot,
} from "./types.js";

interface PendingRequest {
  resolve: (response: RuntimeHostResponse) => void;
  reject: (error: Error) => void;
  timer: NodeJS.Timeout;
  method: RuntimeHostRequest["method"];
}

interface WorkerRetirement {
  child: ChildProcess;
  exit: Promise<void>;
  completion: Promise<void>;
  confirmExit: () => void;
  terminationTimer?: NodeJS.Timeout;
  killTimer?: NodeJS.Timeout;
}

export class IsolatedPiWorkerFactory implements AgentSessionFactory {
  private readonly client: RuntimeWorkerClient;

  constructor(config: AdapterConfig, logger: Logger) {
    this.client = new RuntimeWorkerClient(config, logger);
  }

  async create(options: CreateSessionOptions): Promise<AgentSessionHandle> {
    const proxy = new RuntimeSessionProxy(
      this.client,
      options.acpSessionId,
      options.eventSink,
    );
    this.client.register(proxy);
    try {
      const response = await this.client.request(
        "create",
        options.acpSessionId,
        {
          cwd: options.cwd,
          ...(options.requestedCwd === undefined
            ? {}
            : { requestedCwd: options.requestedCwd }),
          ...(options.systemPrompt === undefined
            ? {}
            : { systemPrompt: options.systemPrompt }),
          ...(options.title === undefined ? {} : { title: options.title }),
          ...(options.persistedSessionFile === undefined
            ? {}
            : { persistedSessionFile: options.persistedSessionFile }),
        },
      );
      const result = response.result as CreateRuntimeResult;
      if (!response.state) {
        throw new Error("Pi runtime create response omitted session state");
      }
      proxy.applyState(response.state);
      proxy.applyResources(result.resources);
      return proxy;
    } catch (error) {
      this.client.unregister(options.acpSessionId);
      throw error;
    }
  }

  async shutdown(): Promise<void> {
    await this.client.shutdown();
  }

  setInvalidationHandler(
    handler: (
      sessionIds: readonly string[],
      error: Error,
    ) => void | Promise<void>,
  ): void {
    this.client.setInvalidationHandler(handler);
  }
}

export class RuntimeWorkerClient {
  private child: ChildProcess | undefined;
  private childSender:
    | {
        child: ChildProcess;
        queue: BoundedIpcSendQueue<RuntimeHostRequest>;
      }
    | undefined;
  private nextId = 1;
  private readonly pending = new Map<number, PendingRequest>();
  private readonly sessions = new Map<string, RuntimeSessionProxy>();
  private shuttingDown = false;
  private shutdownPromise: Promise<void> | undefined;
  private retirement: WorkerRetirement | undefined;
  private invalidationHandler:
    | ((sessionIds: readonly string[], error: Error) => void | Promise<void>)
    | undefined;

  constructor(
    private readonly config: AdapterConfig,
    private readonly logger: Logger,
    private readonly spawnChild?: () => ChildProcess,
  ) {}

  register(proxy: RuntimeSessionProxy): void {
    this.sessions.set(proxy.acpSessionId, proxy);
  }

  unregister(sessionId: string): void {
    this.sessions.delete(sessionId);
  }

  setInvalidationHandler(
    handler: (
      sessionIds: readonly string[],
      error: Error,
    ) => void | Promise<void>,
  ): void {
    this.invalidationHandler = handler;
  }

  async request(
    method: RuntimeHostRequest["method"],
    sessionId?: string,
    params?: Record<string, unknown>,
  ): Promise<RuntimeHostResponse> {
    if (this.shuttingDown && method !== "shutdown") {
      throw new Error("Pi runtime host is shutting down");
    }
    const child = await this.ensureChild();
    const id = this.nextId++;
    const request: RuntimeHostRequest = {
      type: "request",
      id,
      method,
      ...(sessionId === undefined ? {} : { sessionId }),
      ...(params === undefined ? {} : { params }),
    };
    const timeoutMs = requestTimeoutMs(method, this.config);
    const promise = new Promise<RuntimeHostResponse>((resolve, reject) => {
      const timer = setTimeout(() => {
        const pending = this.pending.get(id);
        if (!pending) return;
        const error = new Error(`Pi runtime ${method} request timed out`);
        this.handleChildFailure(child, error);
      }, timeoutMs);
      timer.unref();
      this.pending.set(id, { resolve, reject, timer, method });
    });
    const sender = this.childSender;
    if (!sender || sender.child !== child) {
      this.handleChildFailure(
        child,
        new Error("Pi runtime IPC sender is unavailable"),
      );
    } else {
      sender.queue.enqueue(request);
    }
    return promise;
  }

  shutdown(): Promise<void> {
    this.shutdownPromise ??= this.shutdownInner();
    return this.shutdownPromise;
  }

  private async shutdownInner(): Promise<void> {
    this.shuttingDown = true;
    if (this.retirement) {
      await this.retirement.completion;
      return;
    }
    const child = this.child;
    if (!child) return;
    try {
      await this.request("shutdown");
      await this.beginChildRetirement(
        child,
        new Error("Pi runtime host shut down"),
        false,
        true,
      );
    } catch (error) {
      this.logger.warn("forcing Pi runtime host shutdown", {
        error: error instanceof Error ? error.message : String(error),
      });
      await this.beginChildRetirement(
        child,
        error instanceof Error ? error : new Error(String(error)),
      );
    }
  }

  private async ensureChild(): Promise<ChildProcess> {
    if (this.retirement) await this.retirement.completion;
    if (this.child?.connected) return this.child;
    if (this.shuttingDown) throw new Error("Pi runtime host is shutting down");
    if (this.child) {
      await this.beginChildRetirement(
        this.child,
        new Error("Pi runtime IPC channel disconnected before process exit"),
      );
      if (this.shuttingDown)
        throw new Error("Pi runtime host is shutting down");
    }
    const child = this.spawnChild ? this.spawnChild() : spawnRuntimeHost();
    child.stdout?.on("data", (chunk: Buffer) => {
      process.stderr.write(
        `[buzz-pi-runtime stdout] ${chunk.toString("utf8")}`,
      );
    });
    child.stderr?.on("data", (chunk: Buffer) => {
      process.stderr.write(`[buzz-pi-runtime] ${chunk.toString("utf8")}`);
    });
    child.on("message", (message: RuntimeHostMessage) =>
      this.handleMessage(child, message),
    );
    child.on("error", (error) => this.handleChildFailure(child, error));
    child.on("exit", (code, signal) => {
      this.handleChildExit(child, code, signal);
    });
    this.child = child;
    this.childSender = {
      child,
      queue: new BoundedIpcSendQueue<RuntimeHostRequest>(
        (message, callback) => child.send(message, callback),
        this.config.maxRuntimeIpcQueueMessages,
        this.config.maxRuntimeIpcQueueBytes,
        (error) => this.handleChildFailure(child, error),
      ),
    };
    this.logger.info("started isolated Pi runtime host", {
      pid: child.pid,
      maxSessions: this.config.maxSessions,
    });
    return child;
  }

  private handleMessage(
    child: ChildProcess,
    message: RuntimeHostMessage,
  ): void {
    if (this.child !== child || this.retirement?.child === child) return;
    if (message.type === "response") {
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      clearTimeout(pending.timer);
      if (message.ok) pending.resolve(message);
      else
        pending.reject(
          new Error(message.error?.message ?? "Pi runtime host request failed"),
        );
      return;
    }
    const proxy = this.sessions.get(message.sessionId);
    if (!proxy) return;
    void proxy.handleEvent(message).catch((error: unknown) => {
      this.handleChildFailure(
        child,
        error instanceof Error ? error : new Error(String(error)),
      );
    });
  }

  private rejectAllPending(error: Error): void {
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.pending.clear();
  }

  private handleChildFailure(child: ChildProcess, error: Error): void {
    if (this.child !== child) return;
    void this.beginChildRetirement(child, error, child.pid === undefined);
  }

  private handleChildExit(
    child: ChildProcess,
    code: number | null,
    signal: NodeJS.Signals | null,
  ): void {
    if (this.retirement?.child === child) {
      this.retirement.confirmExit();
      return;
    }
    if (this.child !== child) return;
    void this.beginChildRetirement(
      child,
      new Error(
        `Pi runtime host exited (code=${code ?? "none"}, signal=${signal ?? "none"})`,
      ),
      true,
    );
  }

  private beginChildRetirement(
    child: ChildProcess,
    error: Error,
    alreadyExited = false,
    graceful = false,
  ): Promise<void> {
    if (this.retirement?.child === child) return this.retirement.completion;
    if (this.child !== child) return Promise.resolve();

    this.childSender = undefined;
    // Freeze request deadlines while the generation is fenced. Requests are
    // rejected only after confirmed process exit, so callers cannot release a
    // conversation lease or spawn a replacement while old Pi code can write.
    for (const pending of this.pending.values()) clearTimeout(pending.timer);

    let exited = false;
    let resolveExit: () => void = () => {};
    const exit = new Promise<void>((resolve) => {
      resolveExit = resolve;
    });
    const retirement: WorkerRetirement = {
      child,
      exit,
      completion: Promise.resolve(),
      confirmExit: () => {
        if (exited) return;
        exited = true;
        resolveExit();
      },
    };
    this.retirement = retirement;

    const proxies = [...this.sessions.values()];
    const sessionIds = proxies.map((proxy) => proxy.acpSessionId);
    for (const proxy of proxies) proxy.invalidate(error, exit);
    this.sessions.clear();

    retirement.completion = (async () => {
      await exit;
      if (retirement.terminationTimer)
        clearTimeout(retirement.terminationTimer);
      if (retirement.killTimer) clearTimeout(retirement.killTimer);
      if (this.child === child) this.child = undefined;
      if (child.connected) {
        try {
          child.disconnect();
        } catch {
          // The process exit is already confirmed; a closed IPC channel is OK.
        }
      }
      this.rejectAllPending(error);
      if (sessionIds.length > 0) {
        try {
          await this.invalidationHandler?.(sessionIds, error);
        } catch (handlerError) {
          this.logger.error("Pi runtime invalidation handler failed", {
            error:
              handlerError instanceof Error
                ? handlerError.message
                : String(handlerError),
          });
        }
      }
      if (this.retirement === retirement) this.retirement = undefined;
    })();

    if (alreadyExited) {
      retirement.confirmExit();
      return retirement.completion;
    }

    const graceMs = this.config.runtimeInterruptTimeoutMs;
    const forceKill = (): void => {
      if (exited) return;
      this.logger.warn("Pi runtime host ignored SIGTERM; sending SIGKILL", {
        pid: child.pid,
      });
      this.signalChild(child, "SIGKILL");
    };
    const terminate = (): void => {
      if (exited) return;
      this.signalChild(child, "SIGTERM");
      retirement.killTimer = setTimeout(forceKill, graceMs);
      retirement.killTimer.unref();
    };
    if (graceful) {
      if (child.connected) {
        try {
          child.disconnect();
        } catch {
          // Escalation below remains authoritative.
        }
      }
      retirement.terminationTimer = setTimeout(terminate, graceMs);
      retirement.terminationTimer.unref();
    } else {
      terminate();
    }
    return retirement.completion;
  }

  private signalChild(child: ChildProcess, signal: NodeJS.Signals): void {
    try {
      child.kill(signal);
    } catch (signalError) {
      this.logger.warn("failed to signal Pi runtime host", {
        pid: child.pid,
        signal,
        error:
          signalError instanceof Error
            ? signalError.message
            : String(signalError),
      });
    }
  }
}

function spawnRuntimeHost(): ChildProcess {
  const cliPath = fileURLToPath(new URL("./cli.js", import.meta.url));
  return fork(cliPath, ["--runtime-host"], {
    env: { ...process.env, BUZZ_PI_RUNTIME_HOST: "1" },
    stdio: ["ignore", "pipe", "pipe", "ipc"],
    serialization: "advanced",
  });
}

export class RuntimeSessionProxy implements AgentSessionHandle {
  private state: RuntimeSessionState | undefined;
  private resources: ResourceSnapshot | undefined;
  private busy = false;
  private disposed = false;
  private disposePromise: Promise<void> | undefined;
  private failure: Error | undefined;
  private retirementFence: Promise<void> | undefined;
  private readonly eventTasks = new Set<Promise<void>>();

  constructor(
    private readonly client: RuntimeWorkerClient,
    readonly acpSessionId: string,
    private readonly sink: AdapterEventSink,
  ) {}

  get piSessionId(): string {
    return this.requireState().piSessionId;
  }

  get sessionFile(): string | undefined {
    return this.requireState().sessionFile;
  }

  get cwd(): string {
    return this.requireState().cwd;
  }

  get isBusy(): boolean {
    return this.busy || (this.state?.isBusy ?? false);
  }

  get isValid(): boolean {
    return !this.disposed && this.failure === undefined;
  }

  async prompt(
    text: string,
    images: AcpImageBlock[] = [],
  ): Promise<"end_turn" | "cancelled" | "max_tokens"> {
    this.busy = true;
    try {
      const response = await this.invoke("prompt", { text, images });
      await this.drainEventTasks();
      return response.result as "end_turn" | "cancelled" | "max_tokens";
    } finally {
      this.busy = false;
    }
  }

  async steer(text: string): Promise<void> {
    await this.invoke("steer", { text });
  }

  async abort(): Promise<void> {
    if (this.disposed) return;
    await this.invoke("abort");
    this.busy = false;
  }

  async setModel(modelId: string): Promise<void> {
    await this.invoke("setModel", { modelId });
  }

  async setThinkingLevel(level: string): Promise<void> {
    await this.invoke("setThinkingLevel", { level });
  }

  async reload(): Promise<ResourceSnapshot> {
    const response = await this.invoke("reload");
    const resources = response.result as ResourceSnapshot;
    this.applyResources(resources);
    return resources;
  }

  async reset(): Promise<{
    previousPiSessionId: string;
    resources: ResourceSnapshot;
  }> {
    const response = await this.invoke("reset");
    const result = response.result as {
      previousPiSessionId: string;
      resources: ResourceSnapshot;
    };
    this.applyResources(result.resources);
    return result;
  }

  async replayLifecycleEvents(): Promise<void> {
    await this.invoke("replayLifecycle");
    await this.drainEventTasks();
  }

  getModels(): ModelDescriptor[] {
    return [...this.requireState().models];
  }

  getThinkingLevels(): string[] {
    return [...this.requireState().thinkingLevels];
  }

  getResources(): ResourceSnapshot {
    if (!this.resources)
      throw new Error("Pi runtime resources have not initialized");
    return structuredClone(this.resources);
  }

  getContextSnapshot(): ContextSnapshot {
    return { ...this.requireState().context };
  }

  getUsageSnapshot(): SessionUsageSnapshot {
    return { ...this.requireState().usage };
  }

  dispose(): Promise<void> {
    this.disposePromise ??= this.disposeInner();
    return this.disposePromise;
  }

  private async disposeInner(): Promise<void> {
    try {
      // A worker failure prevents new RPCs but does not erase already-buffered
      // lifecycle handoffs. Drain them into the parent outbox before registry
      // identity is retired, even when the child can no longer be disposed.
      await this.drainEventTasks();
      if (!this.failure) {
        this.disposed = true;
        await this.invoke("dispose");
      }
    } finally {
      this.disposed = true;
      await this.retirementFence;
      this.client.unregister(this.acpSessionId);
    }
  }

  applyState(state: RuntimeSessionState): void {
    this.state = state;
  }

  applyResources(resources: ResourceSnapshot): void {
    this.resources = structuredClone(resources);
  }

  handleEvent(event: RuntimeHostEvent): Promise<void> {
    if (!this.isValid) return Promise.resolve();
    const task = this.handleEventInner(event);
    this.eventTasks.add(task);
    void task.then(
      () => this.eventTasks.delete(task),
      () => this.eventTasks.delete(task),
    );
    return task;
  }

  private async handleEventInner(event: RuntimeHostEvent): Promise<void> {
    if (event.eventType === "session_update") {
      this.sink.sessionUpdate(this.acpSessionId, event.payload);
    } else if (event.eventType === "buzz_session_event") {
      await this.sink.buzzSessionEvent(
        this.acpSessionId,
        event.payload,
        event.deliveryId,
      );
      if (event.deliveryId !== undefined) {
        await this.invoke("ackLifecycle", { deliveryId: event.deliveryId });
      }
    } else {
      this.sink.usageUpdate(
        this.acpSessionId,
        event.payload.usage,
        event.payload.contextLimit,
      );
    }
  }

  private async drainEventTasks(): Promise<void> {
    const failures: unknown[] = [];
    while (this.eventTasks.size > 0) {
      const results = await Promise.allSettled([...this.eventTasks]);
      for (const result of results) {
        if (result.status === "rejected") failures.push(result.reason);
      }
    }
    if (failures.length === 1) throw failures[0];
    if (failures.length > 1) {
      throw new AggregateError(
        failures,
        "Pi runtime lifecycle handoff encountered multiple failures",
      );
    }
  }

  private async invoke(
    method: RuntimeHostRequest["method"],
    params?: Record<string, unknown>,
  ): Promise<RuntimeHostResponse> {
    if (this.disposed && method !== "dispose")
      throw new Error("Pi runtime session is disposed");
    if (this.failure) throw this.failure;
    let response: RuntimeHostResponse;
    try {
      response = await this.client.request(method, this.acpSessionId, params);
    } catch (error) {
      if (
        error instanceof Error &&
        error.message.startsWith("BUZZ_PI_SESSION_INVALIDATED:")
      ) {
        this.failure = error;
        this.busy = false;
        this.client.unregister(this.acpSessionId);
      }
      throw error;
    }
    if (response.state) this.applyState(response.state);
    return response;
  }

  private requireState(): RuntimeSessionState {
    if (this.failure) throw this.failure;
    if (!this.state) throw new Error("Pi runtime session has not initialized");
    return this.state;
  }

  invalidate(error: Error, retirementFence: Promise<void>): void {
    this.failure = error;
    this.retirementFence = retirementFence;
    this.busy = false;
  }
}

export function requestTimeoutMs(
  method: RuntimeHostRequest["method"],
  config: AdapterConfig,
): number {
  if (method === "prompt") return config.runtimeRequestTimeoutMs;
  if (["steer", "abort", "dispose", "shutdown"].includes(method)) {
    return config.runtimeInterruptTimeoutMs;
  }
  return config.runtimeControlTimeoutMs;
}
