/**
 * Serialize microphone on/off requests so the last one asked for is the one
 * that lands.
 *
 * Toggling is a single tap, so two requests overlap easily — and the first
 * unmute is the slowest case of all, because it may wait on a permission
 * prompt and `getUserMedia`. That is exactly when somebody taps again.
 *
 * The provider SDK reconciles most of this already: LiveKit's disable path
 * looks for a publication still in flight and mutes it once it appears. Two
 * gaps are left, and the second is the one that matters here.
 *
 * That wait has a timeout. When it expires the SDK logs and gives up, while
 * the enable it could not find goes on to publish — a microphone left open by
 * a request the member superseded.
 *
 * And the *reported* state is nobody's job but ours. Without a fence, a
 * superseded request's rejection reverts the indicator to muted while a newer
 * request has the microphone genuinely open. A microphone somebody believes is
 * off is not a state-tidiness concern, so this errs toward never claiming
 * "muted" on behalf of a request that no longer speaks for the member.
 */
export type MicRequestQueue = {
  /**
   * Ask for the microphone to be `enabled`.
   *
   * `onFailure` runs only if this request is still the newest when it fails,
   * so a stale rejection cannot describe a newer request's device.
   *
   * Returns the queue's tail, which callers may await in tests. Production
   * does not: the outcome reaches the UI through `onFailure`.
   */
  request: (enabled: boolean, onFailure: () => void) => Promise<void>;
  /**
   * Supersede every in-flight request without issuing a new one.
   *
   * Called when the room changes: a request settling afterwards describes a
   * connection that no longer exists, and must neither reach the SDK nor write
   * state about it.
   */
  supersede: () => void;
};

/**
 * Build a queue that applies microphone requests one at a time.
 *
 * `apply` is the provider call. At most one runs at a time, and a request that
 * is no longer the newest when its turn comes is dropped rather than applied.
 * So the values a member passes through on the way to the one they settle on
 * never reach the device — they are discarded, not queued up to be replayed at
 * it one after another.
 *
 * How many calls a burst costs depends on its timing, and neither number is a
 * promise worth making: taps landing in one tick supersede each other before
 * any call starts and cost a single call, while a tap arriving after a call is
 * already in flight cannot recall it and costs a second.
 */
export function createMicRequestQueue(
  apply: (enabled: boolean) => Promise<void>,
): MicRequestQueue {
  let latest = 0;
  let tail: Promise<void> = Promise.resolve();

  return {
    request(enabled, onFailure) {
      latest += 1;
      const generation = latest;
      tail = tail
        .then(async () => {
          if (latest !== generation) return;
          await apply(enabled);
        })
        .catch(() => {
          if (latest !== generation) return;
          onFailure();
        });
      return tail;
    },
    supersede() {
      latest += 1;
    },
  };
}
