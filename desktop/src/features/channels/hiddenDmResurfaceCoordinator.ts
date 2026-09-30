import type { RelayEvent } from "@/shared/api/types";

type CoordinatorOptions = {
  resurface: (event: RelayEvent) => Promise<unknown>;
  isCurrent: () => boolean;
  onError?: (channelId: string, error: unknown) => void;
};

/**
 * Per-channel coalescing for hidden-DM resurface attempts.
 *
 * The reopen action is idempotent, so concurrent messages for the same DM
 * share one in-flight attempt. A follower that lands while an attempt is
 * running flags it for retry (from the latest event) instead of being
 * dropped, so a failed reopen re-runs rather than leaving the row hidden.
 *
 * A coordinator owns its own pending map, so callers create one per
 * subscription generation: an attempt from a torn-down generation can never
 * delete or coalesce into an entry owned by the live one.
 */
export function createHiddenDmResurfaceCoordinator({
  resurface,
  isCurrent,
  onError,
}: CoordinatorOptions) {
  const pending = new Map<string, { retry: boolean }>();
  const latestEventByChannel = new Map<string, RelayEvent>();

  const attempt = async (channelId: string) => {
    const state = { retry: false };
    pending.set(channelId, state);
    try {
      do {
        state.retry = false;
        const event = latestEventByChannel.get(channelId);
        if (!event) return;
        try {
          await resurface(event);
          return;
        } catch (error) {
          onError?.(channelId, error);
        }
      } while (state.retry && isCurrent());
    } finally {
      pending.delete(channelId);
    }
  };

  return {
    handle(channelId: string, event: RelayEvent) {
      latestEventByChannel.set(channelId, event);
      const existing = pending.get(channelId);
      if (existing) {
        existing.retry = true;
        return;
      }
      void attempt(channelId);
    },
  };
}
