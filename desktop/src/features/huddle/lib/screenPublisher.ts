import type { RelayEvent } from "@/shared/api/types";
import type { VideoBatch } from "./screenShare";

type PublisherDependencies = {
  capture: () => Promise<RelayEvent | null>;
  publish: (event: RelayEvent, isCurrent: () => boolean) => Promise<unknown>;
  fail: (error: unknown) => Promise<void>;
  schedule: (callback: () => void, delay: number) => unknown;
  unschedule: (timer: unknown) => void;
  now?: () => number;
};

function quotaCooldown(error: unknown): number | null {
  if (!(error instanceof Error)) return null;
  const hint = /^rate-limited: quota exceeded; retry in (\d+)s$/.exec(
    error.message,
  );
  if (!hint) return null;
  const seconds = Number(hint[1]);
  return seconds <= 5 ? Math.max(250, seconds * 1000 + 250) : null;
}

/** Drain native encoded batches, never pace video with a browser snapshot timer. */
export function startScreenPublisher(deps: PublisherDependencies): () => void {
  let disposed = false;
  let timer: unknown;
  let refusals = 0;
  let cooldownTotal = 0;
  let captureAfter = 0;
  const now = deps.now ?? Date.now;
  const isCurrent = () => !disposed;
  async function tick() {
    if (disposed) return;
    try {
      const event = await deps.capture();
      if (disposed) return;
      if (event) {
        // Native capture stays bounded during cooldown; do not replay its old queue.
        if (captureAfter > 0) {
          const batch = JSON.parse(event.content) as VideoBatch;
          const timestamp = batch.frames.at(-1)?.timestamp;
          if (
            batch.type !== "video" ||
            !batch.share ||
            !Number.isSafeInteger(batch.share.started_at) ||
            batch.share.started_at <= 0 ||
            timestamp === undefined ||
            !Number.isSafeInteger(timestamp) ||
            timestamp < 0
          )
            throw new Error("Screen capture returned invalid video");
          if (batch.share.started_at + timestamp / 1000 < captureAfter) {
            timer = deps.schedule(() => void tick(), 0);
            return;
          }
        }
        try {
          await deps.publish(event, isCurrent);
        } catch (error) {
          const delay = quotaCooldown(error);
          if (
            disposed ||
            delay === null ||
            refusals >= 6 ||
            cooldownTotal + delay > 15_000
          )
            throw error;
          refusals++;
          cooldownTotal += delay;
          captureAfter = now() + delay;
          timer = deps.schedule(() => void tick(), delay);
          return;
        }
        refusals = 0;
        cooldownTotal = 0;
        captureAfter = 0;
      }
      if (!disposed) timer = deps.schedule(() => void tick(), 0);
    } catch (error) {
      if (disposed) return;
      disposed = true;
      await deps.fail(error);
    }
  }
  void tick();
  return () => {
    disposed = true;
    if (timer !== undefined) deps.unschedule(timer);
  };
}
