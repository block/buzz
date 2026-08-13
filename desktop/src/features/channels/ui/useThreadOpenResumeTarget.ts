import * as React from "react";
import {
  selectThreadResumeTargetId,
  type ThreadResumeTargetInput,
} from "@/features/channels/lib/threadResumeTarget";

type UseThreadOpenResumeTargetResult = {
  /** Reply to centre on this open, or null. Stable for the whole open. */
  threadResumeScrollTargetId: string | null;
  /** Retires the latch once the scroll has been performed. */
  onThreadResumeTargetConsumed: () => void;
};

/**
 * One-shot "resume where I left off" target, latched per thread open.
 *
 * The decision is captured **during render**, not in an effect: the panel's
 * own layout effect bottom-pins on its first commit, so a target seeded from
 * an effect always lands too late. This mirrors `openFrontierRef` in
 * `useChannelUnreadState`, which writes during render for the same reason.
 *
 * A captured `null` is latched too. That is a decision ("nothing was unread
 * when this opened"), and it is what stops a live reply arriving into a
 * fully-read thread from re-deriving a target and jumping the reader.
 */
export function useThreadOpenResumeTarget(
  input: ThreadResumeTargetInput,
): UseThreadOpenResumeTargetResult {
  const capturedRef = React.useRef(new Map<string, string | null>());
  const consumedRef = React.useRef(new Set<string>());
  const headId = input.openThreadHeadId;

  // Mirror for the stable consume callback, which must not take `headId` as a
  // dependency (it is handed to a memoized child).
  const headIdRef = React.useRef<string | null>(null);
  headIdRef.current = headId;

  // Consuming flips a ref, which alone would not re-render — and the target
  // must actually reach null downstream, or `pinTargetCentered` stays true and
  // `releasePinnedCenter` never runs, freezing the reader in place.
  const [, bumpConsumed] = React.useReducer((n: number) => n + 1, 0);

  if (headId !== null && !capturedRef.current.has(headId)) {
    // Readiness is `hasReplies`, not a query-pending flag: a forum thread's
    // query is disabled and stays pending forever, while replies can already
    // be resolved from the channel timeline. Until the first reply renders we
    // simply do not latch, and re-evaluate on the next render.
    if (input.hasReplies) {
      capturedRef.current.set(headId, selectThreadResumeTargetId(input));
    }
  }

  // Drop both entries when the thread closes or switches, so re-opening the
  // same thread re-snapshots against the read state as it is then.
  React.useEffect(() => {
    if (headId === null) return;
    return () => {
      capturedRef.current.delete(headId);
      consumedRef.current.delete(headId);
    };
  }, [headId]);

  const onThreadResumeTargetConsumed = React.useCallback(() => {
    const id = headIdRef.current;
    if (id === null || consumedRef.current.has(id)) return;
    consumedRef.current.add(id);
    bumpConsumed();
  }, []);

  const threadResumeScrollTargetId =
    headId !== null && !consumedRef.current.has(headId)
      ? (capturedRef.current.get(headId) ?? null)
      : null;

  return { threadResumeScrollTargetId, onThreadResumeTargetConsumed };
}
