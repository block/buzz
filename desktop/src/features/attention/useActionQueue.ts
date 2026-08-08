import * as React from "react";
import { toast } from "sonner";

export const UNDO_WINDOW_MS = 5_000;

export type QueuedActionInput = {
  /** Attention item id — one pending action per item at a time. */
  itemId: string;
  /** Toast headline, e.g. "Marked done — reply queued". */
  toastLabel: string;
  /** Zone change to apply immediately (optimistic). */
  apply: () => void;
  /** Reverse of `apply`, used by Undo and by send failure. */
  revert: () => void;
  /** Posts the threaded reply. Called once, only after the undo window. */
  send: () => Promise<unknown>;
};

export type QueuedBatchInput = {
  /** One toast for the whole batch; Undo restores every item in it. */
  toastLabel: string;
  items: Array<Omit<QueuedActionInput, "toastLabel">>;
};

type PendingAction = {
  timer: ReturnType<typeof setTimeout>;
};

/**
 * Optimistic action queue with a 5-second undo window.
 *
 * Each action applies its zone change immediately, then waits
 * UNDO_WINDOW_MS before posting the reply. Undo cancels the timer —
 * nothing is ever published and then deleted. A send failure reverts
 * the zone change and surfaces an error toast. At most one action can
 * be pending per item, so double-clicks and key repeats cannot
 * double-post.
 */
export function useActionQueue() {
  const pendingRef = React.useRef<Map<string, PendingAction>>(new Map());
  const [pendingIds, setPendingIds] = React.useState<ReadonlySet<string>>(
    () => new Set(),
  );

  const markPending = React.useCallback((itemId: string, pending: boolean) => {
    setPendingIds((prev) => {
      const next = new Set(prev);
      if (pending) {
        next.add(itemId);
      } else {
        next.delete(itemId);
      }
      return next;
    });
  }, []);

  const queueBatch = React.useCallback(
    ({ toastLabel, items }: QueuedBatchInput) => {
      const fresh = items.filter(
        (item) => !pendingRef.current.has(item.itemId),
      );
      if (fresh.length === 0) {
        return false;
      }
      for (const item of fresh) {
        item.apply();
      }

      const finishAll = () => {
        for (const item of fresh) {
          pendingRef.current.delete(item.itemId);
          markPending(item.itemId, false);
        }
      };

      const timer = setTimeout(() => {
        finishAll();
        for (const item of fresh) {
          item.send().catch(() => {
            toast.error("Could not post your reply. The item was restored.");
            item.revert();
          });
        }
      }, UNDO_WINDOW_MS);

      for (const item of fresh) {
        pendingRef.current.set(item.itemId, { timer });
        markPending(item.itemId, true);
      }

      toast(toastLabel, {
        action: {
          label: "Undo",
          onClick: () => {
            if (!fresh.some((item) => pendingRef.current.has(item.itemId))) {
              return;
            }
            clearTimeout(timer);
            finishAll();
            for (const item of fresh) {
              item.revert();
            }
          },
        },
        duration: UNDO_WINDOW_MS,
      });
      return true;
    },
    [markPending],
  );

  const queueAction = React.useCallback(
    ({ itemId, toastLabel, apply, revert, send }: QueuedActionInput) =>
      queueBatch({ toastLabel, items: [{ itemId, apply, revert, send }] }),
    [queueBatch],
  );

  return { pendingIds, queueAction, queueBatch };
}

export type ActionQueue = ReturnType<typeof useActionQueue>;
