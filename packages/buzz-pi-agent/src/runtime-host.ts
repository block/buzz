import { loadConfig } from "./config.js";
import { BoundedIpcSendQueue } from "./ipc-send-queue.js";
import { createLogger } from "./logger.js";
import { PiAgentSessionFactory } from "./pi-runtime.js";
import type {
  CreateRuntimeResult,
  RuntimeHostEvent,
  RuntimeHostRequest,
  RuntimeHostResponse,
  RuntimeSessionState,
} from "./runtime-host-protocol.js";
import type {
  AcpImageBlock,
  AdapterEventSink,
  AgentSessionHandle,
  CreateSessionOptions,
} from "./types.js";

export class PerKeyRequestQueue {
  private readonly queues = new Map<string, Promise<void>>();

  async run<T>(key: string, operation: () => Promise<T>): Promise<T> {
    const prior = this.queues.get(key) ?? Promise.resolve();
    let resolveCurrent: () => void = () => {};
    const current = new Promise<void>((resolve) => {
      resolveCurrent = resolve;
    });
    this.queues.set(key, current);
    await prior.catch(() => {});
    try {
      return await operation();
    } finally {
      resolveCurrent();
      if (this.queues.get(key) === current) this.queues.delete(key);
    }
  }
}

export async function runRuntimeHost(): Promise<void> {
  if (!process.send)
    throw new Error("runtime host must be started with an IPC channel");
  const config = loadConfig();
  const logger = createLogger(config.logLevel);
  const factory = new PiAgentSessionFactory(config, logger);
  const sessions = new Map<string, AgentSessionHandle>();
  const sessionQueues = new PerKeyRequestQueue();
  let shuttingDown = false;

  const disposeAll = async (): Promise<void> => {
    if (shuttingDown) return;
    shuttingDown = true;
    await Promise.allSettled(
      [...sessions.values()].map(async (session) => {
        if (session.isBusy) await session.abort();
        await session.dispose();
      }),
    );
    sessions.clear();
  };

  let transportFailed = false;
  const failTransport = (error: Error): void => {
    if (transportFailed) return;
    transportFailed = true;
    logger.error("Pi runtime IPC transport poisoned", {
      error: error.message,
    });
    void disposeAll().finally(() => process.exit(1));
  };
  const sender = new BoundedIpcSendQueue<
    RuntimeHostResponse | RuntimeHostEvent
  >(
    (message, callback) => {
      if (!process.connected || !process.send) {
        callback(new Error("Pi runtime IPC channel is disconnected"));
        return false;
      }
      return process.send(message, callback);
    },
    config.maxRuntimeIpcQueueMessages,
    config.maxRuntimeIpcQueueBytes,
    failTransport,
  );
  const send = (message: RuntimeHostResponse | RuntimeHostEvent): void => {
    sender.enqueue(message);
  };

  const retireInvalidSession = async (
    sessionId: string,
    session: AgentSessionHandle,
    cause: unknown,
  ): Promise<Error> => {
    sessions.delete(sessionId);
    await session.dispose().catch((error: unknown) => {
      logger.error("failed to dispose invalidated Pi runtime session", {
        sessionId,
        error: error instanceof Error ? error.message : String(error),
      });
    });
    const causeMessage = cause instanceof Error ? cause.message : String(cause);
    if (causeMessage.startsWith("BUZZ_PI_SESSION_INVALIDATED:")) {
      return cause instanceof Error ? cause : new Error(causeMessage);
    }
    return new Error(
      "BUZZ_PI_SESSION_INVALIDATED: Pi resource generation failed; create a fresh session",
      { cause },
    );
  };

  const sink: AdapterEventSink = {
    sessionUpdate(sessionId, update) {
      send({
        type: "event",
        sessionId,
        eventType: "session_update",
        payload: update,
      });
    },
    buzzSessionEvent(sessionId, event, deliveryId) {
      send({
        type: "event",
        sessionId,
        eventType: "buzz_session_event",
        ...(deliveryId === undefined ? {} : { deliveryId }),
        payload: event,
      });
    },
    usageUpdate(sessionId, usage, contextLimit) {
      send({
        type: "event",
        sessionId,
        eventType: "usage_update",
        payload: { usage, contextLimit },
      });
    },
  };

  process.on("message", (raw: unknown) => {
    if (!isRuntimeHostRequest(raw)) return;
    void dispatch(raw).catch((error: unknown) => {
      send({
        type: "response",
        id: raw.id,
        ok: false,
        error: serializeError(error),
      });
    });
  });
  process.on("disconnect", () => {
    void disposeAll().finally(() => process.exit(0));
  });
  process.on("SIGTERM", () => {
    void disposeAll().finally(() => process.exit(0));
  });

  async function handle(request: RuntimeHostRequest): Promise<void> {
    if (request.method === "shutdown") {
      await disposeAll();
      send({
        type: "response",
        id: request.id,
        ok: true,
        result: { shutdown: true },
      });
      return;
    }
    if (shuttingDown) throw new Error("Pi runtime host is shutting down");
    const sessionId = request.sessionId;
    if (!sessionId) throw new Error(`${request.method} requires sessionId`);

    if (request.method === "create") {
      const prior = sessions.get(sessionId);
      if (prior) {
        // Remove ownership before teardown: even a broken extension dispose
        // must not pin a dead same-ID session in the host map forever.
        sessions.delete(sessionId);
        await prior.dispose();
      }
      const params = request.params ?? {};
      const systemPrompt = optionalString(params.systemPrompt);
      const title = optionalString(params.title);
      const requestedCwd = optionalString(params.requestedCwd);
      const persistedSessionFile = optionalString(params.persistedSessionFile);
      const options: CreateSessionOptions = {
        acpSessionId: sessionId,
        cwd: requiredString(params.cwd, "cwd"),
        eventSink: sink,
        ...(requestedCwd === undefined ? {} : { requestedCwd }),
        ...(systemPrompt === undefined ? {} : { systemPrompt }),
        ...(title === undefined ? {} : { title }),
        ...(persistedSessionFile === undefined ? {} : { persistedSessionFile }),
      };
      const session = await factory.create(options);
      sessions.set(sessionId, session);
      const state = snapshot(session);
      const result: CreateRuntimeResult = {
        resources: session.getResources(),
      };
      send({
        type: "response",
        id: request.id,
        ok: true,
        result,
        state,
      });
      return;
    }

    const session = sessions.get(sessionId);
    if (!session) throw new Error(`Unknown runtime session ${sessionId}`);
    let result: unknown;
    try {
      switch (request.method) {
        case "prompt":
          result = await session.prompt(
            requiredString(request.params?.text, "text"),
            asImages(request.params?.images),
          );
          break;
        case "steer":
          await session.steer(requiredString(request.params?.text, "text"));
          result = { steered: true };
          break;
        case "abort":
          await session.abort();
          result = { aborted: true };
          break;
        case "setModel":
          await session.setModel(
            requiredString(request.params?.modelId, "modelId"),
          );
          result = { changed: true };
          break;
        case "setThinkingLevel":
          await session.setThinkingLevel(
            requiredString(request.params?.level, "level"),
          );
          result = { changed: true };
          break;
        case "reload":
          result = await session.reload();
          break;
        case "reset":
          result = await session.reset();
          break;
        case "replayLifecycle":
          await session.replayLifecycleEvents?.();
          result = { replayed: true };
          break;
        case "ackLifecycle": {
          const deliveryId = requiredString(
            request.params?.deliveryId,
            "deliveryId",
          );
          if (!session.acknowledgeLifecycleEvent) {
            throw new Error(
              "Pi runtime session does not support lifecycle acknowledgements",
            );
          }
          await session.acknowledgeLifecycleEvent(deliveryId);
          result = { acknowledged: true };
          break;
        }
        case "dispose":
          if (session.isBusy) await session.abort();
          await session.dispose();
          sessions.delete(sessionId);
          send({
            type: "response",
            id: request.id,
            ok: true,
            result: { disposed: true },
          });
          return;
        default:
          throw new Error(
            `Unsupported runtime host method ${request.method satisfies never}`,
          );
      }
    } catch (error) {
      if (!session.isValid) {
        throw await retireInvalidSession(sessionId, session, error);
      }
      throw error;
    }
    if (!session.isValid) {
      throw await retireInvalidSession(
        sessionId,
        session,
        new Error("Pi resource generation became invalid"),
      );
    }
    send({
      type: "response",
      id: request.id,
      ok: true,
      result,
      state: snapshot(session),
    });
  }

  async function dispatch(request: RuntimeHostRequest): Promise<void> {
    // Steering and abort must be able to interrupt an active prompt. Every
    // other state mutation is ordered per session; distinct sessions remain
    // fully concurrent.
    if (
      request.method === "shutdown" ||
      request.method === "abort" ||
      request.method === "steer" ||
      request.method === "dispose" ||
      !request.sessionId
    ) {
      await handle(request);
      return;
    }
    const sessionId = request.sessionId;
    await sessionQueues.run(sessionId, () => handle(request));
  }
}

function snapshot(session: AgentSessionHandle): RuntimeSessionState {
  return {
    piSessionId: session.piSessionId,
    ...(session.sessionFile === undefined
      ? {}
      : { sessionFile: session.sessionFile }),
    cwd: session.cwd,
    isBusy: session.isBusy,
    models: session.getModels(),
    thinkingLevels: session.getThinkingLevels(),
    context: session.getContextSnapshot(),
    usage: session.getUsageSnapshot(),
  };
}

function isRuntimeHostRequest(value: unknown): value is RuntimeHostRequest {
  return (
    typeof value === "object" &&
    value !== null &&
    "type" in value &&
    value.type === "request" &&
    "id" in value &&
    typeof value.id === "number" &&
    "method" in value &&
    typeof value.method === "string"
  );
}

function requiredString(value: unknown, name: string): string {
  if (typeof value !== "string" || value.trim() === "") {
    throw new Error(`${name} must be a non-empty string`);
  }
  return value;
}

function optionalString(value: unknown): string | undefined {
  return typeof value === "string" && value !== "" ? value : undefined;
}

function asImages(value: unknown): AcpImageBlock[] {
  if (!Array.isArray(value)) return [];
  return value.filter(
    (item): item is AcpImageBlock =>
      typeof item === "object" &&
      item !== null &&
      "type" in item &&
      item.type === "image" &&
      "data" in item &&
      typeof item.data === "string" &&
      "mimeType" in item &&
      typeof item.mimeType === "string",
  );
}

function serializeError(error: unknown): { message: string } {
  const message = error instanceof Error ? error.message : String(error);
  return {
    message: message.length <= 2_000 ? message : `${message.slice(0, 2_000)}…`,
  };
}
