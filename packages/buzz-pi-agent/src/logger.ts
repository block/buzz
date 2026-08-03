import type { AdapterConfig } from "./config.js";
import type { Logger } from "./types.js";

const levels = { debug: 10, info: 20, warn: 30, error: 40 } as const;

export function createLogger(level: AdapterConfig["logLevel"]): Logger {
  const threshold = levels[level];
  const log = (
    name: keyof typeof levels,
    message: string,
    fields?: Record<string, unknown>,
  ): void => {
    if (levels[name] < threshold) return;
    const record = {
      timestamp: new Date().toISOString(),
      level: name,
      component: "buzz-pi-agent",
      message,
      ...fields,
    };
    process.stderr.write(`${JSON.stringify(record)}\n`);
  };

  return {
    debug: (message, fields) => log("debug", message, fields),
    info: (message, fields) => log("info", message, fields),
    warn: (message, fields) => log("warn", message, fields),
    error: (message, fields) => log("error", message, fields),
  };
}

/**
 * ACP reserves stdout for NDJSON. Save the only permitted writer, then route
 * console and ordinary stdout writes to stderr. Pi extensions execute in an
 * isolated child as an additional boundary, but this guard protects the ACP
 * parent and accidental writes in adapter code too.
 */
export function guardProtocolStdout(): (line: string) => Promise<void> {
  const stdout = process.stdout;
  const protocolWrite = stdout.write.bind(stdout) as (
    line: string,
    callback: (error?: Error | null) => void,
  ) => boolean;
  const stderrWrite = process.stderr.write.bind(process.stderr);

  process.stdout.write = stderrWrite as typeof process.stdout.write;
  console.log = (...args: unknown[]) => writeConsole(args);
  console.info = (...args: unknown[]) => writeConsole(args);
  console.debug = (...args: unknown[]) => writeConsole(args);
  console.warn = (...args: unknown[]) => writeConsole(args);
  console.error = (...args: unknown[]) => writeConsole(args);

  return (line: string): Promise<void> =>
    new Promise<void>((resolve, reject) => {
      let callbackDone = false;
      let drainDone = true;
      let writeReturned = false;
      let settled = false;

      const cleanup = (): void => {
        stdout.off("error", onError);
        stdout.off("drain", onDrain);
      };
      const finish = (): void => {
        if (settled || !writeReturned || !callbackDone || !drainDone) return;
        settled = true;
        cleanup();
        resolve();
      };
      const fail = (error: Error): void => {
        if (settled) return;
        settled = true;
        cleanup();
        reject(error);
      };
      const onError = (error: Error): void => fail(error);
      const onDrain = (): void => {
        drainDone = true;
        finish();
      };

      stdout.once("error", onError);
      try {
        const accepted = protocolWrite(line, (error) => {
          if (error) {
            fail(error);
            return;
          }
          callbackDone = true;
          finish();
        });
        if (!accepted) {
          drainDone = false;
          stdout.once("drain", onDrain);
        }
        writeReturned = true;
        finish();
      } catch (error) {
        fail(error instanceof Error ? error : new Error(String(error)));
      }
    });
}

function writeConsole(args: unknown[]): void {
  process.stderr.write(`${args.map(formatValue).join(" ")}\n`);
}

function formatValue(value: unknown): string {
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}
