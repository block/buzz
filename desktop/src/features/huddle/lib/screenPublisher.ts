import type { RelayEvent } from "@/shared/api/types";
import { PublishCanceledError } from "@/shared/api/relayEventPublisher";
import type { VideoBatch } from "./screenShare";
import { HuddleMediaClock } from "./huddleMediaClock";

type PublisherDependencies = {
  capture: (refreshKeyframe: boolean) => Promise<RelayEvent | null>;
  publish: (event: RelayEvent, isCurrent: () => boolean) => Promise<unknown>;
  fail: (error: unknown) => Promise<void>;
  schedule?: (callback: () => void, delay: number) => unknown;
  unschedule?: (timer: unknown) => void;
  now?: () => number;
};

/** Bounded acknowledgement window; capture itself remains single-flight. */
export const MAX_VIDEO_PUBLICATIONS = 4;
const MAX_CAPTURE_AGE_MS = 500;
const MAX_STALE_RECOVERIES = 8;

function capturedAt(event: RelayEvent): number {
  const batch = JSON.parse(event.content) as VideoBatch;
  const timestamp = batch.frames?.at(-1)?.timestamp;
  if (
    batch.type !== "video" ||
    !batch.share ||
    !Number.isSafeInteger(batch.share.started_at) ||
    batch.share.started_at <= 0 ||
    timestamp === undefined ||
    !Number.isSafeInteger(timestamp) ||
    timestamp < 0
  )
    throw new Error("Video capture returned invalid video");
  return batch.share.started_at + timestamp / 1000;
}

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
  let timerAt = 0;
  let refusals = 0;
  let cooldownTotal = 0;
  let captureAfter = 0;
  let cooldownUntil = 0;
  let generation = 0;
  let inFlight = 0;
  let capturing = false;
  let refreshKeyframe = false;
  let staleRecoveries = 0;
  let clock: HuddleMediaClock | undefined;
  const now = deps.now ?? Date.now;
  let schedule = deps.schedule;
  let unschedule = deps.unschedule;
  const stop = () => {
    disposed = true;
    generation++;
    try {
      if (timer !== undefined) unschedule?.(timer);
    } finally {
      timer = undefined;
      clock?.dispose();
    }
  };
  const failed = async (error: unknown) => {
    if (disposed) return;
    try {
      stop();
    } finally {
      await deps.fail(error);
    }
  };
  try {
    if (!schedule && !unschedule) {
      clock = new HuddleMediaClock((error) => {
        void failed(error);
      });
      const mediaClock = clock;
      schedule = (callback, delay) => mediaClock.schedule(callback, delay);
      unschedule = (timer) => mediaClock.unschedule(timer);
    }
    if (!schedule || !unschedule)
      throw new Error("Huddle publication requires a cancellable clock");
  } catch (error) {
    void failed(error);
    return stop;
  }
  const scheduleNext = schedule;
  const wake = () => {
    if (disposed || capturing || inFlight >= MAX_VIDEO_PUBLICATIONS) return;
    try {
      const deadline = Math.max(now(), cooldownUntil);
      if (
        timer !== undefined &&
        timerAt >= cooldownUntil &&
        timerAt <= deadline
      )
        return;
      if (timer !== undefined) unschedule?.(timer);
      timerAt = deadline;
      timer = scheduleNext(
        () => {
          timer = undefined;
          void tick();
        },
        Math.max(0, cooldownUntil - now()),
      );
    } catch (error) {
      void failed(error);
    }
  };
  const retirePackets = () => {
    generation++;
    refreshKeyframe = true;
  };
  const stale = async () => {
    retirePackets();
    if (++staleRecoveries >= MAX_STALE_RECOVERIES)
      await failed(new Error("Video capture could not recover fresh frames"));
  };
  async function tick() {
    if (disposed || capturing || inFlight >= MAX_VIDEO_PUBLICATIONS) return;
    capturing = true;
    const epoch = generation;
    try {
      const refresh = refreshKeyframe;
      refreshKeyframe = false;
      const event = await deps.capture(refresh);
      if (disposed || epoch !== generation) return;
      if (event) {
        const captured = capturedAt(event);
        const fresh = () =>
          captured >= captureAfter && now() - captured <= MAX_CAPTURE_AGE_MS;
        if (!fresh()) {
          await stale();
          return;
        }
        const isCurrent = () => !disposed && epoch === generation && fresh();
        inFlight++;
        void Promise.resolve()
          .then(() => deps.publish(event, isCurrent))
          .then(
            () => {
              if (disposed || epoch !== generation) return;
              refusals = 0;
              cooldownTotal = 0;
              captureAfter = 0;
              staleRecoveries = 0;
            },
            async (error: unknown) => {
              if (disposed) return;
              if (error instanceof PublishCanceledError && !isCurrent()) {
                if (epoch === generation) await stale();
                return;
              }
              const delay = quotaCooldown(error);
              if (delay !== null && epoch !== generation) return;
              if (
                delay === null ||
                refusals >= 6 ||
                cooldownTotal + delay > 15_000
              ) {
                await failed(error);
                return;
              }
              refusals++;
              cooldownTotal += delay;
              cooldownUntil = now() + delay;
              captureAfter = cooldownUntil;
              retirePackets();
            },
          )
          .finally(() => {
            inFlight--;
            wake();
          });
      }
    } catch (error) {
      await failed(error);
    } finally {
      capturing = false;
      wake();
    }
  }
  void tick();
  return stop;
}
