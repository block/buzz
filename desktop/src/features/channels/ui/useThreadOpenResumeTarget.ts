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
 *
 * The latch must not fire while replies are still arriving. A thread reopened
 * from cache renders one reply off the channel timeline first, and at that
 * point the unread marker has not been recomputed — so the latch would capture
 * `null`, which this hook keeps as a deliberate decision. The real target
 * arrives with the rest of the replies and is then ignored forever, leaving
 * the reader bottom-pinned: exactly what this resume exists to replace. That is
 * why `hasReplies` is expected to fold in the caller's settled-query gate, and
 * why `hasReadHistory` rides that same gate rather than one of its own.
 *
 * `fetchStatus` is the caller's signal there, not `isPending`. A refetch over
 * cached data reports `isPending: false` while still in flight, so pending
 * alone lets the early render through. `"idle"` means nothing is on the wire —
 * true once the replies have landed, and true for a forum thread whose query is
 * disabled and whose replies resolve from the channel timeline instead. Paired
 * with a reply-count check, a query that has not started yet is excluded as
 * well, since it has no replies to latch onto.
 *
 * `hasReadHistory` has to arrive already frozen, from `useChannelUnreadState`,
 * and it cannot be re-derived here from `getMessageReadAt`. That accessor is
 * hierarchical — `msg:<id>` resolves to max(own marker, active channel marker)
 * — so once the channel has been opened every reply reports a read time, and a
 * snapshot taken at this hook's first sight of the replies claims read history
 * for a thread the reader has never opened, resuming them to the top: the exact
 * defect the guard exists to prevent. Measured, not theorised.
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
