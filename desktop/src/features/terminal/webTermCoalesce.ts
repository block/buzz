// Web Buzz Term: keep typing and key-repeat responsive on slow devices (iPhone).
// - Frames: echo frames that arrive faster than the screen paints are merged (rows by line, latest cursor) and handed to
//   React once per animation frame, instead of one full Bootstrap/Substrate render per frame. Nothing is dropped: a
//   partial frame's rows survive the merge, and a frame for a different viewport flushes the pending one first.
// - Input: the first key goes out at once; keys during the next few ms (iOS backspace repeat, fast typing) ride one
//   message, so the socket and the desktop see a short burst instead of a queue.
import type { TerminalFrame } from "./terminalRenderer";

type FrameLike = TerminalFrame & { sequence: number };
type DeliveryLike<F extends FrameLike> = { frame: F; acknowledge: () => Promise<void> };

const sameViewport = (a: TerminalFrame, b: TerminalFrame) =>
  a.viewport.generation === b.viewport.generation &&
  a.viewport.columns === b.viewport.columns &&
  a.viewport.screenLines === b.viewport.screenLines;

/** b applied on top of a, or null when they can't be merged (viewport changed). */
export function mergeFrames<F extends FrameLike>(a: F, b: F): F | null {
  if (b.full) return b;
  if (!sameViewport(a, b)) return null;
  const byLine = new Map<number, TerminalFrame["rows"][number]>();
  for (const row of a.rows) byLine.set(row.line, row);
  for (const row of b.rows) byLine.set(row.line, row);
  return { ...b, full: a.full, rows: [...byLine.values()].sort((x, y) => x.line - y.line) };
}

export function createFrameCoalescer<F extends FrameLike>(
  emit: (delivery: DeliveryLike<F>) => void,
  schedule: (fn: () => void) => void = (fn) => window.requestAnimationFrame(fn),
) {
  let pending: DeliveryLike<F> | null = null;
  let scheduled = false;
  const flush = () => {
    scheduled = false;
    const d = pending;
    pending = null;
    if (d) emit(d);
  };
  return {
    push(delivery: DeliveryLike<F>) {
      if (pending) {
        const merged = mergeFrames(pending.frame, delivery.frame);
        if (merged) {
          pending = { frame: merged, acknowledge: delivery.acknowledge };
        } else {
          flush();
          pending = delivery;
        }
      } else {
        pending = delivery;
      }
      if (!scheduled) {
        scheduled = true;
        schedule(flush);
      }
    },
    flush,
  };
}

/** Leading-edge input batching: send now, then batch whatever arrives within `windowMs`. */
export function createInputCoalescer(
  send: (text: string) => void,
  windowMs = 12,
  setTimer: (fn: () => void, ms: number) => unknown = (fn, ms) => setTimeout(fn, ms),
) {
  let buf = "";
  let open = false;
  const close = () => {
    if (buf) {
      const out = buf;
      buf = "";
      send(out);
      setTimer(close, windowMs); // keep batching while a burst continues
      return;
    }
    open = false;
  };
  return (text: string) => {
    if (!text) return;
    if (!open) {
      open = true;
      send(text);
      setTimer(close, windowMs);
      return;
    }
    buf += text;
  };
}
