/**
 * Strictly ordered, bounded queue for Node IPC frames. `process.send()`'s
 * callback is the flow-control boundary; a false return simply confirms that
 * waiting for that callback is mandatory before submitting the next frame.
 */
export class BoundedIpcSendQueue<T> {
  private readonly queue: Array<{ message: T; bytes: number }> = [];
  private queuedBytes = 0;
  private sending = false;
  private fatalError: Error | undefined;

  constructor(
    private readonly rawSend: (
      message: T,
      callback: (error?: Error | null) => void,
    ) => boolean,
    private readonly maxQueuedMessages: number,
    private readonly maxQueuedBytes: number,
    private readonly onFatal: (error: Error) => void,
  ) {}

  enqueue(message: T): boolean {
    if (this.fatalError) return false;
    let normalized: { message: T; bytes: number };
    try {
      normalized = normalizeAndMeasureIpcFrame(message, this.maxQueuedBytes);
    } catch (error) {
      this.poison(
        new Error(
          `failed to size IPC frame: ${error instanceof Error ? error.message : String(error)}`,
        ),
      );
      return false;
    }
    if (
      this.queue.length + 1 > this.maxQueuedMessages ||
      this.queuedBytes + normalized.bytes > this.maxQueuedBytes
    ) {
      this.poison(
        new Error(
          `IPC queue saturated (${this.queue.length + 1} messages, ${this.queuedBytes + normalized.bytes} bytes)`,
        ),
      );
      return false;
    }
    this.queue.push(normalized);
    this.queuedBytes += normalized.bytes;
    this.flush();
    return true;
  }

  private flush(): void {
    if (this.sending || this.fatalError) return;
    const next = this.queue[0];
    if (!next) return;
    this.sending = true;
    try {
      // Waiting for the exact frame's callback handles both true and false
      // return values without reordering the subsequent frame.
      void this.rawSend(next.message, (error) => {
        if (error) {
          this.poison(error);
          return;
        }
        if (this.fatalError) return;
        this.queue.shift();
        this.queuedBytes -= next.bytes;
        this.sending = false;
        queueMicrotask(() => this.flush());
      });
    } catch (error) {
      this.poison(error instanceof Error ? error : new Error(String(error)));
    }
  }

  private poison(error: Error): void {
    if (this.fatalError) return;
    this.fatalError = error;
    this.sending = false;
    this.queue.length = 0;
    this.queuedBytes = 0;
    this.onFatal(error);
  }
}

const MAX_IPC_FRAME_NODES = 50_000;
const MAX_IPC_FRAME_DEPTH = 128;
const MAX_IPC_ENUMERATED_KEYS = 8_192;

function normalizeAndMeasureIpcFrame<T>(
  value: T,
  maxBytes: number,
): { message: T; bytes: number } {
  // Runtime IPC is intentionally a JSON-shaped protocol even though Node's
  // channel uses the advanced serializer. Clone accepted values before they
  // enter the queue so later mutation cannot invalidate the size calculation,
  // and reject advanced/non-plain values instead of sizing them as `{}`.
  const ancestors = new WeakSet<object>();
  let nodes = 0;
  let bytes = 0;
  const addBytes = (amount: number): void => {
    bytes += amount;
    if (bytes > maxBytes) {
      throw new Error(`IPC frame exceeds ${maxBytes} bytes`);
    }
  };
  const visit = (item: unknown, depth: number): unknown => {
    nodes += 1;
    if (nodes > MAX_IPC_FRAME_NODES) {
      throw new Error(`IPC frame exceeds ${MAX_IPC_FRAME_NODES} nodes`);
    }
    if (depth > MAX_IPC_FRAME_DEPTH) {
      throw new Error(`IPC frame exceeds ${MAX_IPC_FRAME_DEPTH} levels`);
    }
    if (typeof item === "string") {
      addBytes(jsonStringByteLength(item));
      return item;
    }
    if (typeof item === "number") {
      if (!Number.isFinite(item)) {
        throw new Error("IPC frame contains a non-finite number");
      }
      const normalized = Object.is(item, -0) ? 0 : item;
      addBytes(Buffer.byteLength(String(normalized)));
      return normalized;
    }
    if (typeof item === "boolean") {
      addBytes(item ? 4 : 5);
      return item;
    }
    if (item === null) {
      addBytes(4);
      return null;
    }
    if (typeof item !== "object") {
      throw new Error(`IPC frame contains unsupported ${typeof item} data`);
    }
    const unsupportedKind = unsupportedIpcObjectKind(item);
    if (unsupportedKind) {
      throw new Error(`IPC frame contains unsupported ${unsupportedKind} data`);
    }
    if (ancestors.has(item)) throw new Error("IPC frame contains a cycle");
    ancestors.add(item);
    try {
      if (Array.isArray(item)) {
        addBytes(2);
        const result: unknown[] = [];
        for (let index = 0; index < item.length; index += 1) {
          let descriptor: PropertyDescriptor | undefined;
          try {
            descriptor = Object.getOwnPropertyDescriptor(item, index);
          } catch (error) {
            throw new Error(
              `IPC frame array item could not be inspected: ${errorMessage(error)}`,
            );
          }
          if (!descriptor) throw new Error("IPC frame contains a sparse array");
          if (!("value" in descriptor)) {
            throw new Error("IPC frame contains an accessor property");
          }
          if (index > 0) addBytes(1);
          result.push(visit(descriptor.value, depth + 1));
        }
        assertArrayHasNoEnumerableDecorations(item);
        return result;
      }

      let prototype: object | null;
      try {
        prototype = Object.getPrototypeOf(item);
      } catch (error) {
        throw new Error(
          `IPC frame object could not be inspected: ${errorMessage(error)}`,
        );
      }
      if (prototype !== Object.prototype && prototype !== null) {
        throw new Error("IPC frame contains a non-plain object");
      }

      addBytes(2);
      const result: Record<string, unknown> = {};
      let entries = 0;
      let enumeratedKeys = 0;
      try {
        for (const key in item) {
          enumeratedKeys += 1;
          if (enumeratedKeys > MAX_IPC_ENUMERATED_KEYS) {
            throw new Error(
              `IPC frame exceeds ${MAX_IPC_ENUMERATED_KEYS} enumerated keys`,
            );
          }
          if (!Object.hasOwn(item, key)) continue;
          if (entries >= MAX_IPC_FRAME_NODES) {
            throw new Error(`IPC frame exceeds ${MAX_IPC_FRAME_NODES} entries`);
          }
          if (entries > 0) addBytes(1);
          addBytes(jsonStringByteLength(key) + 1);
          let descriptor: PropertyDescriptor | undefined;
          try {
            descriptor = Object.getOwnPropertyDescriptor(item, key);
          } catch (error) {
            throw new Error(
              `IPC frame property could not be inspected: ${errorMessage(error)}`,
            );
          }
          if (!descriptor || !("value" in descriptor)) {
            throw new Error("IPC frame contains an accessor property");
          }
          result[key] = visit(descriptor.value, depth + 1);
          entries += 1;
        }
      } catch (error) {
        if (error instanceof Error) throw error;
        throw new Error(
          `IPC frame object could not be inspected: ${errorMessage(error)}`,
        );
      }
      return result;
    } finally {
      ancestors.delete(item);
    }
  };
  const message = visit(value, 0) as T;
  return { message, bytes };
}

function unsupportedIpcObjectKind(value: object): string | undefined {
  if (value instanceof ArrayBuffer) return "ArrayBuffer";
  if (
    typeof SharedArrayBuffer !== "undefined" &&
    value instanceof SharedArrayBuffer
  ) {
    return "SharedArrayBuffer";
  }
  if (ArrayBuffer.isView(value)) return "typed-array/view";
  if (value instanceof Map) return "Map";
  if (value instanceof Set) return "Set";
  if (value instanceof Date) return "Date";
  return undefined;
}

function assertArrayHasNoEnumerableDecorations(value: unknown[]): void {
  let entries = 0;
  let enumeratedKeys = 0;
  try {
    for (const key in value) {
      enumeratedKeys += 1;
      if (enumeratedKeys > MAX_IPC_ENUMERATED_KEYS) {
        throw new Error(
          `IPC frame exceeds ${MAX_IPC_ENUMERATED_KEYS} enumerated keys`,
        );
      }
      if (!Object.hasOwn(value, key)) continue;
      if (key !== String(entries) || entries >= value.length) {
        throw new Error("IPC frame contains a decorated array");
      }
      entries += 1;
    }
  } catch (error) {
    if (error instanceof Error && error.message.includes("decorated array")) {
      throw error;
    }
    throw new Error(
      `IPC frame array could not be inspected: ${errorMessage(error)}`,
    );
  }
  if (entries !== value.length) {
    throw new Error("IPC frame contains a sparse array");
  }
}

function jsonStringByteLength(value: string): number {
  let bytes = 2;
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code === 0x22 || code === 0x5c) {
      bytes += 2;
    } else if (code <= 0x1f) {
      bytes += [0x08, 0x09, 0x0a, 0x0c, 0x0d].includes(code) ? 2 : 6;
    } else if (code <= 0x7f) {
      bytes += 1;
    } else if (code <= 0x7ff) {
      bytes += 2;
    } else if (code >= 0xd800 && code <= 0xdbff) {
      const next = value.charCodeAt(index + 1);
      if (next >= 0xdc00 && next <= 0xdfff) {
        bytes += 4;
        index += 1;
      } else {
        bytes += 6;
      }
    } else if (code >= 0xdc00 && code <= 0xdfff) {
      bytes += 6;
    } else {
      bytes += 3;
    }
  }
  return bytes;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
