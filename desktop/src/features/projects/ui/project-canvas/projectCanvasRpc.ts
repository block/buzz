import {
  isMessageWithinSizeLimit,
  PROJECT_CANVAS_COMMAND_RATE_LIMIT,
  PROJECT_CANVAS_COMMAND_RATE_WINDOW_MS,
  PROJECT_CANVAS_MAX_CONCURRENT_SUBSCRIPTIONS,
  PROJECT_CANVAS_MAX_PORT_MESSAGE_BYTES,
  PROJECT_CANVAS_OPEN_RATE_LIMIT,
  PROJECT_CANVAS_OPEN_RATE_WINDOW_MS,
  PROJECT_CANVAS_PROTOCOL_VERSION,
  ProjectCanvasMessageRateLimiter,
  type ProjectCanvasCapability,
  type ProjectCanvasChildMessage,
  type ProjectCanvasRpcError,
} from "./projectCanvasProtocol";
import {
  ProjectCanvasBrokerError,
  type ProjectCanvasBroker,
} from "./projectCanvasBroker";

/**
 * Per-frame RPC dispatcher. Owns the category budgets (concurrent
 * subscriptions, command and open rate windows), response size enforcement,
 * and teardown fencing: after `dispose()` no late async completion can post
 * to the (already closed) port. Budget violations answer the individual
 * request with an error — they never kill the frame.
 */

export type ProjectCanvasRpcSessionOptions = {
  broker: ProjectCanvasBroker | null;
  capabilities: readonly ProjectCanvasCapability[];
  loadId: string;
  nonce: string;
  now?: () => number;
  onCommandSettled?: (commandName: string, error: string | null) => void;
  post: (message: object) => void;
};

export type ProjectCanvasRpcSession = {
  dispose: () => void;
  handle: (
    message: Exclude<
      ProjectCanvasChildMessage,
      { type: "canvas.layout" } | { type: "canvas.rendered" }
    >,
  ) => void;
};

function rpcError(
  code: ProjectCanvasRpcError["code"],
  message: string,
): ProjectCanvasRpcError {
  return { code, message };
}

function toRpcError(error: unknown): ProjectCanvasRpcError {
  if (error instanceof ProjectCanvasBrokerError) return error.toRpcError();
  return rpcError(
    "failed",
    error instanceof Error ? error.message : "Canvas request failed.",
  );
}

export function createProjectCanvasRpcSession(
  options: ProjectCanvasRpcSessionOptions,
): ProjectCanvasRpcSession {
  const now = options.now ?? (() => performance.now());
  const commandLimiter = new ProjectCanvasMessageRateLimiter(
    PROJECT_CANVAS_COMMAND_RATE_LIMIT,
    PROJECT_CANVAS_COMMAND_RATE_WINDOW_MS,
  );
  const openLimiter = new ProjectCanvasMessageRateLimiter(
    PROJECT_CANVAS_OPEN_RATE_LIMIT,
    PROJECT_CANVAS_OPEN_RATE_WINDOW_MS,
  );
  const subscriptions = new Map<string, () => void>();
  let disposed = false;

  const envelope = {
    loadId: options.loadId,
    nonce: options.nonce,
    protocolVersion: PROJECT_CANVAS_PROTOCOL_VERSION,
  } as const;

  const post = (message: object) => {
    if (disposed) return;
    options.post({ ...envelope, ...message });
  };

  const postSized = (message: object, fallback: object) => {
    const sized = { ...envelope, ...message };
    if (
      isMessageWithinSizeLimit(sized, PROJECT_CANVAS_MAX_PORT_MESSAGE_BYTES)
    ) {
      post(message);
      return true;
    }
    post(fallback);
    return false;
  };

  const brokerUnavailable = rpcError(
    "unavailable",
    "Canvas data access is unavailable in this context.",
  );

  const handle: ProjectCanvasRpcSession["handle"] = (message) => {
    if (disposed) return;
    switch (message.type) {
      case "canvas.query": {
        const { queryId } = message;
        if (!options.broker) {
          post({ error: brokerUnavailable, queryId, type: "host.queryResult" });
          return;
        }
        void options.broker
          .query(message.query.name, message.query.params, options.capabilities)
          .then(
            (result) => {
              postSized(
                { queryId, result, type: "host.queryResult" },
                {
                  error: rpcError(
                    "too-large",
                    "Canvas query result exceeds the port message limit.",
                  ),
                  queryId,
                  type: "host.queryResult",
                },
              );
            },
            (error: unknown) => {
              post({
                error: toRpcError(error),
                queryId,
                type: "host.queryResult",
              });
            },
          );
        return;
      }
      case "canvas.subscribe": {
        const { subscriptionId } = message;
        const end = (error: ProjectCanvasRpcError) => {
          post({ error, subscriptionId, type: "host.subscriptionEnded" });
        };
        if (!options.broker) {
          end(brokerUnavailable);
          return;
        }
        if (subscriptions.has(subscriptionId)) {
          end(rpcError("invalid-params", "Subscription id is already in use."));
          return;
        }
        if (subscriptions.size >= PROJECT_CANVAS_MAX_CONCURRENT_SUBSCRIPTIONS) {
          end(
            rpcError(
              "rate-limited",
              `Canvas frames are limited to ${PROJECT_CANVAS_MAX_CONCURRENT_SUBSCRIPTIONS} concurrent subscriptions.`,
            ),
          );
          return;
        }
        // Register a placeholder before subscribing: the broker pushes the
        // initial result synchronously and the update path only delivers for
        // registered ids.
        subscriptions.set(subscriptionId, () => {});
        try {
          const unsubscribe = options.broker.subscribe(
            message.query.name,
            message.query.params,
            options.capabilities,
            (result) => {
              if (disposed || !subscriptions.has(subscriptionId)) return;
              const delivered = postSized(
                { result, subscriptionId, type: "host.subscriptionUpdate" },
                {
                  error: rpcError(
                    "too-large",
                    "Canvas subscription update exceeds the port message limit.",
                  ),
                  subscriptionId,
                  type: "host.subscriptionEnded",
                },
              );
              if (!delivered) {
                subscriptions.get(subscriptionId)?.();
                subscriptions.delete(subscriptionId);
              }
            },
          );
          if (subscriptions.has(subscriptionId)) {
            subscriptions.set(subscriptionId, unsubscribe);
          } else {
            // The initial update was already terminal (oversized); detach.
            unsubscribe();
          }
        } catch (error) {
          subscriptions.delete(subscriptionId);
          end(toRpcError(error));
          return;
        }
        return;
      }
      case "canvas.unsubscribe": {
        const unsubscribe = subscriptions.get(message.subscriptionId);
        if (!unsubscribe) return;
        subscriptions.delete(message.subscriptionId);
        unsubscribe();
        return;
      }
      case "canvas.command": {
        const { commandId } = message;
        const commandName = message.command.name;
        if (!options.broker) {
          post({
            commandId,
            error: brokerUnavailable,
            type: "host.commandResult",
          });
          return;
        }
        if (!commandLimiter.accept(now())) {
          post({
            commandId,
            error: rpcError(
              "rate-limited",
              `Canvas frames are limited to ${PROJECT_CANVAS_COMMAND_RATE_LIMIT} commands per minute.`,
            ),
            type: "host.commandResult",
          });
          return;
        }
        void options.broker
          .command(commandName, message.command.params, options.capabilities)
          .then(
            () => {
              post({ commandId, ok: true, type: "host.commandResult" });
              options.onCommandSettled?.(commandName, null);
            },
            (error: unknown) => {
              const failure = toRpcError(error);
              post({ commandId, error: failure, type: "host.commandResult" });
              options.onCommandSettled?.(commandName, failure.message);
            },
          );
        return;
      }
      case "canvas.open": {
        const { openId } = message;
        if (!options.broker) {
          post({ error: brokerUnavailable, openId, type: "host.openResult" });
          return;
        }
        if (!openLimiter.accept(now())) {
          post({
            error: rpcError(
              "rate-limited",
              `Canvas frames are limited to ${PROJECT_CANVAS_OPEN_RATE_LIMIT} navigations per ${PROJECT_CANVAS_OPEN_RATE_WINDOW_MS / 1_000} seconds.`,
            ),
            openId,
            type: "host.openResult",
          });
          return;
        }
        void options.broker.open(message.target, options.capabilities).then(
          () => {
            post({ ok: true, openId, type: "host.openResult" });
          },
          (error: unknown) => {
            post({ error: toRpcError(error), openId, type: "host.openResult" });
          },
        );
        return;
      }
    }
  };

  return {
    dispose: () => {
      if (disposed) return;
      disposed = true;
      for (const unsubscribe of subscriptions.values()) unsubscribe();
      subscriptions.clear();
    },
    handle,
  };
}
