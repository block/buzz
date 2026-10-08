/**
 * Run foreground-resume work only after the activation event has returned,
 * the browser has painted one frame, and a following task gets a turn.
 *
 * A single timeout is not enough: it can run before the activating click is
 * delivered. The task -> frame -> task sequence gives WebKit an interaction
 * boundary before query refresh, reconnect, and polling resume work begins.
 */
export function scheduleAfterForegroundReady(callback: () => void): () => void {
  if (typeof window === "undefined") {
    callback();
    return () => {};
  }

  let cancelled = false;
  let frameId: number | null = null;
  let trailingTaskId: number | null = null;

  const leadingTaskId = window.setTimeout(() => {
    if (cancelled) return;
    const afterFrame = () => {
      if (cancelled) return;
      trailingTaskId = window.setTimeout(() => {
        if (!cancelled) callback();
      }, 0);
    };

    if (typeof window.requestAnimationFrame === "function") {
      frameId = window.requestAnimationFrame(afterFrame);
    } else {
      trailingTaskId = window.setTimeout(() => {
        if (!cancelled) callback();
      }, 0);
    }
  }, 0);

  return () => {
    cancelled = true;
    window.clearTimeout(leadingTaskId);
    if (frameId !== null && typeof window.cancelAnimationFrame === "function") {
      window.cancelAnimationFrame(frameId);
    }
    if (trailingTaskId !== null) window.clearTimeout(trailingTaskId);
  };
}
