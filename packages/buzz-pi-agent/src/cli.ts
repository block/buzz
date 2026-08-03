#!/usr/bin/env node
import { AcpServer } from "./server.js";
import { loadConfig } from "./config.js";
import { ConversationStore } from "./conversation-store.js";
import { createLogger, guardProtocolStdout } from "./logger.js";
import { IsolatedPiWorkerFactory } from "./runtime-worker.js";
import { runRuntimeHost } from "./runtime-host.js";
import { SessionRegistry } from "./session-registry.js";
import { NdjsonWriter } from "./wire.js";

const VERSION = "0.1.0";

async function runAdapter(): Promise<void> {
  const config = loadConfig();
  const protocolWrite = guardProtocolStdout();
  const logger = createLogger(config.logLevel);
  let server: AcpServer | undefined;
  let transportFailureTimer: NodeJS.Timeout | undefined;
  const writer = new NdjsonWriter(protocolWrite, logger, {
    maxQueuedMessages: config.maxOutputQueueMessages,
    maxQueuedBytes: config.maxOutputQueueBytes,
    onFatal(error) {
      process.exitCode = 1;
      process.stdin.destroy(error);
      void server?.shutdown().catch((shutdownError: unknown) => {
        logger.error("failed to shut down after ACP transport failure", {
          error:
            shutdownError instanceof Error
              ? shutdownError.message
              : String(shutdownError),
        });
      });
      transportFailureTimer ??= setTimeout(
        () => process.exit(1),
        Math.max(5_000, config.runtimeInterruptTimeoutMs * 3),
      );
      transportFailureTimer.unref();
    },
  });
  const workerFactory = new IsolatedPiWorkerFactory(config, logger);
  const conversations = new ConversationStore(config, logger);
  const serverSink = new DeferredEventSink();
  const registry = new SessionRegistry(
    workerFactory,
    conversations,
    config,
    serverSink,
    logger,
  );
  server = new AcpServer(
    process.stdin,
    writer,
    registry,
    config,
    logger,
    workerFactory,
  );
  serverSink.bind(server);

  let shuttingDown = false;
  const shutdown = (signal: string): void => {
    if (shuttingDown) return;
    shuttingDown = true;
    logger.info("received shutdown signal", { signal });
    void server.shutdown().finally(() => {
      process.exitCode = signal === "SIGINT" ? 130 : 143;
      process.stdin.destroy();
    });
  };
  process.on("SIGINT", () => shutdown("SIGINT"));
  process.on("SIGTERM", () => shutdown("SIGTERM"));
  if (process.platform !== "win32")
    process.on("SIGHUP", () => shutdown("SIGHUP"));

  try {
    await server.run();
  } catch (error) {
    if (shuttingDown) return;
    logger.error("adapter terminated with an error", {
      error: error instanceof Error ? error.message : String(error),
    });
    process.exitCode = 1;
  } finally {
    if (transportFailureTimer) clearTimeout(transportFailureTimer);
  }
}

class DeferredEventSink {
  private target: AcpServer | undefined;

  bind(target: AcpServer): void {
    this.target = target;
  }

  sessionUpdate(sessionId: string, update: Record<string, unknown>): void {
    this.requireTarget().sessionUpdate(sessionId, update);
  }

  buzzSessionEvent(
    sessionId: string,
    event: Parameters<AcpServer["buzzSessionEvent"]>[1],
    deliveryId?: Parameters<AcpServer["buzzSessionEvent"]>[2],
  ): ReturnType<AcpServer["buzzSessionEvent"]> {
    return this.requireTarget().buzzSessionEvent(sessionId, event, deliveryId);
  }

  usageUpdate(
    sessionId: string,
    usage: Parameters<AcpServer["usageUpdate"]>[1],
    contextLimit: number,
  ): void {
    this.requireTarget().usageUpdate(sessionId, usage, contextLimit);
  }

  private requireTarget(): AcpServer {
    if (!this.target)
      throw new Error("ACP event sink was used before server initialization");
    return this.target;
  }
}

if (process.argv.includes("--help") || process.argv.includes("-h")) {
  process.stdout.write(`buzz-pi-agent ${VERSION}

ACP adapter that runs the Pi coding agent inside Buzz.

Usage:
  buzz-pi-agent                 Serve ACP over stdin/stdout
  buzz-pi-agent --help          Show this help
  buzz-pi-agent --version       Show the adapter version

Buzz injects BUZZ_RELAY_URL, BUZZ_PRIVATE_KEY, and BUZZ_AUTH_TAG. See the
package README for context, persistence, trust, timeout, and retention options.
`);
} else if (process.argv.includes("--version") || process.argv.includes("-V")) {
  process.stdout.write(`${VERSION}\n`);
} else if (process.argv.includes("--runtime-host")) {
  await runRuntimeHost();
} else {
  await runAdapter();
}
