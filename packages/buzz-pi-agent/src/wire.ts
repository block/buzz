import type { Readable } from "node:stream";
import type {
  JsonRpcId,
  JsonRpcInbound,
  Logger,
  OutputWriter,
} from "./types.js";

export const PARSE_ERROR = -32700;
export const INVALID_REQUEST = -32600;
export const METHOD_NOT_FOUND = -32601;
export const INVALID_PARAMS = -32602;
export const INTERNAL_ERROR = -32603;
export const AGENT_CONTEXT_LIMIT = -32042;
export const AGENT_OVERLOADED = -32043;
export const AGENT_SESSION_STORAGE_LIMIT = -32044;
export const AGENT_SESSION_INVALIDATED = -32045;

export class JsonRpcError extends Error {
  constructor(
    readonly code: number,
    message: string,
    readonly data?: unknown,
  ) {
    super(message);
    this.name = "JsonRpcError";
  }
}

export class NdjsonWriter implements OutputWriter {
  private readonly queue: Array<{ line: string; bytes: number }> = [];
  private queuedBytes = 0;
  private flushing = false;
  private closed = false;
  private fatalError: Error | undefined;
  private readonly idleWaiters = new Set<{
    resolve: () => void;
    reject: (error: Error) => void;
  }>();

  constructor(
    private readonly rawWrite: (line: string) => void | Promise<void>,
    private readonly logger: Logger,
    private readonly options: {
      maxQueuedMessages?: number;
      maxQueuedBytes?: number;
      onFatal?: (error: Error) => void;
    } = {},
  ) {}

  write(value: unknown): void {
    if (this.fatalError) return;
    if (this.closed) {
      this.poison(new Error("ACP stdout write attempted after close"));
      return;
    }
    let line: string;
    try {
      line = `${JSON.stringify(value)}\n`;
    } catch (error) {
      this.poison(
        new Error(
          `failed to serialize protocol message: ${errorMessage(error)}`,
        ),
      );
      return;
    }
    const bytes = Buffer.byteLength(line);
    const maxQueuedMessages = this.options.maxQueuedMessages ?? 2_048;
    const maxQueuedBytes = this.options.maxQueuedBytes ?? 16 * 1_024 * 1_024;
    if (
      this.queue.length + 1 > maxQueuedMessages ||
      this.queuedBytes + bytes > maxQueuedBytes
    ) {
      this.poison(
        new Error(
          `ACP stdout queue saturated (${this.queue.length + 1} messages, ${this.queuedBytes + bytes} bytes)`,
        ),
      );
      return;
    }
    this.queue.push({ line, bytes });
    this.queuedBytes += bytes;
    this.flush();
  }

  async end(): Promise<void> {
    this.closed = true;
    if (this.fatalError) throw this.fatalError;
    if (this.flushing || this.queue.length > 0) {
      await new Promise<void>((resolve, reject) => {
        this.idleWaiters.add({ resolve, reject });
      });
    }
    if (this.fatalError) throw this.fatalError;
  }

  private flush(): void {
    if (this.flushing || this.fatalError) return;
    this.flushing = true;
    void (async () => {
      try {
        while (this.queue.length > 0 && !this.fatalError) {
          const next = this.queue[0];
          if (!next) break;
          await this.rawWrite(next.line);
          if (this.fatalError) break;
          this.queue.shift();
          this.queuedBytes -= next.bytes;
        }
      } catch (error) {
        this.poison(
          new Error(`failed to write protocol message: ${errorMessage(error)}`),
        );
      } finally {
        this.flushing = false;
        this.settleIdleWaiters();
      }
    })();
  }

  private poison(error: Error): void {
    if (this.fatalError) return;
    this.fatalError = error;
    this.queue.length = 0;
    this.queuedBytes = 0;
    this.logger.error("ACP stdout transport poisoned", {
      error: error.message,
    });
    try {
      this.options.onFatal?.(error);
    } catch (callbackError) {
      this.logger.error("ACP stdout fatal callback failed", {
        error: errorMessage(callbackError),
      });
    }
    this.settleIdleWaiters();
  }

  private settleIdleWaiters(): void {
    if (!this.fatalError && (this.flushing || this.queue.length > 0)) return;
    for (const waiter of this.idleWaiters) {
      if (this.fatalError) waiter.reject(this.fatalError);
      else waiter.resolve();
    }
    this.idleWaiters.clear();
  }
}

export async function* readNdjson(
  input: Readable,
  maxLineBytes: number,
): AsyncGenerator<string> {
  let segments: Buffer[] = [];
  let lineBytes = 0;
  for await (const rawChunk of input) {
    const chunk = Buffer.isBuffer(rawChunk)
      ? rawChunk
      : Buffer.from(String(rawChunk));
    let offset = 0;
    while (offset < chunk.byteLength) {
      const newline = chunk.indexOf(0x0a, offset);
      const end = newline === -1 ? chunk.byteLength : newline;
      const length = end - offset;
      if (lineBytes + length > maxLineBytes) {
        throw new JsonRpcError(
          INVALID_REQUEST,
          `NDJSON line exceeds ${maxLineBytes} bytes`,
        );
      }
      if (length > 0) {
        segments.push(chunk.subarray(offset, end));
        lineBytes += length;
      }
      if (newline === -1) break;

      const frame =
        segments.length === 0
          ? Buffer.alloc(0)
          : segments.length === 1
            ? (segments[0] ?? Buffer.alloc(0))
            : Buffer.concat(segments, lineBytes);
      yield frame.toString("utf8").replace(/\r$/, "");
      segments = [];
      lineBytes = 0;
      offset = newline + 1;
    }
  }
  if (lineBytes > 0) {
    throw new JsonRpcError(PARSE_ERROR, "unterminated NDJSON frame at EOF");
  }
}

export function parseInbound(line: string): JsonRpcInbound {
  let value: unknown;
  try {
    value = JSON.parse(line);
  } catch (error) {
    throw new JsonRpcError(PARSE_ERROR, `Invalid JSON: ${errorMessage(error)}`);
  }
  if (
    !isRecord(value) ||
    value.jsonrpc !== "2.0" ||
    typeof value.method !== "string"
  ) {
    throw new JsonRpcError(
      INVALID_REQUEST,
      "Expected a JSON-RPC 2.0 request or notification",
    );
  }
  if ("id" in value && !isJsonRpcId(value.id)) {
    throw new JsonRpcError(
      INVALID_REQUEST,
      "JSON-RPC id must be a string, number, or null",
    );
  }
  return value as unknown as JsonRpcInbound;
}

export function response(
  id: JsonRpcId,
  result: unknown,
): Record<string, unknown> {
  return { jsonrpc: "2.0", id, result };
}

export function errorResponse(
  id: JsonRpcId,
  error: unknown,
): Record<string, unknown> {
  const rpcError =
    error instanceof JsonRpcError
      ? error
      : new JsonRpcError(INTERNAL_ERROR, errorMessage(error));
  return {
    jsonrpc: "2.0",
    id,
    error: {
      code: rpcError.code,
      message: rpcError.message,
      ...(rpcError.data === undefined ? {} : { data: rpcError.data }),
    },
  };
}

export function notification(
  method: string,
  params: unknown,
): Record<string, unknown> {
  return { jsonrpc: "2.0", method, params };
}

export function asRecord(
  value: unknown,
  name: string,
): Record<string, unknown> {
  if (!isRecord(value))
    throw new JsonRpcError(INVALID_PARAMS, `${name} must be an object`);
  return value;
}

export function requiredString(
  value: Record<string, unknown>,
  key: string,
): string {
  const item = value[key];
  if (typeof item !== "string" || item.trim() === "") {
    throw new JsonRpcError(INVALID_PARAMS, `${key} must be a non-empty string`);
  }
  return item;
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isJsonRpcId(value: unknown): value is JsonRpcId {
  return (
    value === null || typeof value === "string" || typeof value === "number"
  );
}

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
