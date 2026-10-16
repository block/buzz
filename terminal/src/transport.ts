import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import type { HostMessage, Requests } from "./protocol.ts";

/** The app's transport seam, also used by the explicit offline demo. */
export interface Transport {
  start(receive: (message: HostMessage) => void): void;
  request<K extends keyof Requests>(
    method: K,
    params: Requests[K],
  ): Promise<unknown>;
  close(): void;
}

/** A failed request, including uncertain deliveries that must retain their key. */
export class HostError extends Error {
  readonly code: string;
  readonly eventId?: string;
  constructor(code: string, message: string, eventId?: string) {
    super(message);
    this.code = code;
    this.eventId = eventId;
  }
}

/** Starts one local signing host. No shell or plugin can receive keys via this API. */
export class HostTransport implements Transport {
  private child?: ChildProcessWithoutNullStreams;
  private nextId = 0;
  private pending = new Map<
    string,
    {
      resolve: (value: unknown) => void;
      reject: (error: Error) => void;
      timer: NodeJS.Timeout;
    }
  >();
  private receive?: (message: HostMessage) => void;
  private buffer = "";
  private stderr = "";
  private stopped = false;
  private readonly executable: string;

  constructor(executable: string) {
    this.executable = executable;
  }

  start(receive: (message: HostMessage) => void): void {
    this.receive = receive;
    this.child = spawn(this.executable, [], { stdio: "pipe" });
    delete process.env.BUZZ_PRIVATE_KEY;
    delete process.env.BUZZ_AUTH_TAG;
    this.child.stdout.setEncoding("utf8");
    this.child.stderr.setEncoding("utf8");
    this.child.stderr.on("data", (text: string) => {
      this.stderr = (this.stderr + text).slice(-4096);
    });
    this.child.stdout.on("data", (text: string) => this.consume(text));
    this.child.on("error", () =>
      this.fail("Cannot start buzz-terminal-host. Run just terminal-build."),
    );
    this.child.stdin.on("error", () =>
      this.fail("The relay host stopped accepting requests."),
    );
    this.child.on("exit", () => {
      if (!this.stopped)
        this.fail(
          this.stderr.trim() ||
            "The relay host exited. Restart the client to reconnect.",
        );
    });
  }

  private consume(text: string): void {
    this.buffer += text;
    while (this.buffer.includes("\n")) {
      const end = this.buffer.indexOf("\n");
      if (end > 1024 * 1024) {
        this.fail("The relay host exceeded the protocol frame limit.");
        return;
      }
      const line = this.buffer.slice(0, end);
      this.buffer = this.buffer.slice(end + 1);
      try {
        const message = JSON.parse(line) as HostMessage;
        if ("id" in message) {
          const pending = this.pending.get(message.id);
          if (!pending) continue;
          this.pending.delete(message.id);
          clearTimeout(pending.timer);
          if ("error" in message) {
            pending.reject(
              new HostError(
                message.error.code,
                message.error.message,
                message.error.eventId,
              ),
            );
          } else pending.resolve(message.result);
        } else this.receive?.(message);
      } catch {
        this.fail("Invalid response from the relay host.");
        return;
      }
    }
    if (this.buffer.length > 1024 * 1024)
      this.fail("The relay host exceeded the protocol frame limit.");
  }

  request<K extends keyof Requests>(
    method: K,
    params: Requests[K],
  ): Promise<unknown> {
    if (!this.child || this.stopped)
      return Promise.reject(
        new HostError("disconnected", "Relay host is not running."),
      );
    if (this.pending.size >= 64)
      return Promise.reject(
        new HostError("capacity", "Too many pending relay requests."),
      );
    const id = String(++this.nextId);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(
          new HostError(
            method === "sendMessage" ? "uncertain" : "timeout",
            "Relay request timed out. Your draft and send target are retained.",
          ),
        );
      }, 45_000);
      this.pending.set(id, { resolve, reject, timer });
      this.child?.stdin.write(`${JSON.stringify({ id, method, params })}\n`);
    });
  }

  private fail(message: string): void {
    if (this.stopped) return;
    this.close();
    this.receive?.({ type: "connection", status: "failed", message });
  }

  close(): void {
    this.stopped = true;
    for (const request of this.pending.values()) {
      clearTimeout(request.timer);
      request.reject(
        new HostError(
          "uncertain",
          "The relay host stopped. Delivery may be unknown.",
        ),
      );
    }
    this.pending.clear();
    this.child?.stdin.end();
    // A wedged network connection must not outlive the terminal session.
    const child = this.child;
    const timer = setTimeout(() => child?.kill("SIGTERM"), 1000);
    timer.unref();
    child?.once("exit", () => clearTimeout(timer));
  }
}
