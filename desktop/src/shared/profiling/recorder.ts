// Temporary renderer profiling harness (never merges) — ring-buffered recorder.
//
// Records are appended to a bounded in-memory ring buffer and flushed as JSONL
// to a local file (via the `append_profiling_log` Tauri command) every
// `FLUSH_INTERVAL_MS` and on `pagehide`. The buffer is bounded so an unflushed
// or failed sink can never grow the harness into the thing it measures.

import type { ProfileEvent, ProfileRecord } from "@/shared/profiling/types";

/** Max records held between flushes. Oldest are dropped once exceeded. */
export const RING_CAPACITY = 4096;
/** Flush cadence. */
export const FLUSH_INTERVAL_MS = 30_000;

/** Sink receives already-serialized JSONL lines; returns once persisted. */
export type ProfileSink = (lines: string[]) => Promise<void>;

export type RecorderClock = {
  now: () => number;
  wall: () => number;
};

/**
 * Bounded recorder. `record()` stamps the shared envelope and pushes into the
 * ring; `flush()` drains the ring through the sink. Drops on overflow are
 * counted and surfaced as a synthetic field on the next flush's first line so
 * the analyzer can detect (rare) self-pressure.
 */
export class ProfileRecorder {
  private readonly buffer: ProfileRecord[] = [];
  private droppedSinceFlush = 0;
  private flushing = false;
  private readonly startedAt: number;
  private readonly sid: string;
  private readonly sink: ProfileSink;
  private readonly clock: RecorderClock;

  constructor(
    sid: string,
    sink: ProfileSink,
    clock: RecorderClock = {
      now: () => performance.now(),
      wall: () => Date.now(),
    },
  ) {
    this.sid = sid;
    this.sink = sink;
    this.clock = clock;
    this.startedAt = clock.now();
  }

  record(event: ProfileEvent): void {
    const now = this.clock.now();
    const full = {
      ...event,
      t: now,
      wall: this.clock.wall(),
      sid: this.sid,
      up: now - this.startedAt,
    } as ProfileRecord;

    this.buffer.push(full);
    if (this.buffer.length > RING_CAPACITY) {
      this.buffer.shift();
      this.droppedSinceFlush += 1;
    }
  }

  /** Current buffered record count — used by tests and the pending watchdog. */
  size(): number {
    return this.buffer.length;
  }

  async flush(): Promise<void> {
    // Serialize flushes; a slow sink must not let two drains race the buffer.
    if (this.flushing || this.buffer.length === 0) {
      return;
    }
    this.flushing = true;
    const drained = this.buffer.splice(0, this.buffer.length);
    const dropped = this.droppedSinceFlush;
    this.droppedSinceFlush = 0;

    try {
      const lines = drained.map((record, index) =>
        index === 0 && dropped > 0
          ? JSON.stringify({ ...record, dropped })
          : JSON.stringify(record),
      );
      await this.sink(lines);
    } catch {
      // Sink failure must not crash the app being profiled. The dropped count
      // is intentionally not restored — the records are already gone.
    } finally {
      this.flushing = false;
    }
  }
}
