import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  isAppFocused,
  subscribeAppFocus,
} from "@/shared/lib/useDocumentVisible";

/** Reuse Buzz's live cache updates; coalesce bursts without starving a busy feed. */
export function usePulseRefresh({
  enabled,
  scope,
  refresh,
}: {
  enabled: boolean;
  scope: string;
  refresh: () => Promise<unknown>;
}) {
  const queryClient = useQueryClient();
  const refreshLatest = React.useEffectEvent(refresh);
  // biome-ignore lint/correctness/useExhaustiveDependencies: a source or identity change retires the previous refresh generation.
  React.useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    let dirty = false;
    let running = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const previous = new WeakMap<object, unknown>();
    const schedule = () => {
      dirty = true;
      if (disposed || running || timer !== undefined || !isAppFocused()) return;
      timer = setTimeout(() => {
        timer = undefined;
        if (disposed || !isAppFocused()) return;
        dirty = false;
        running = true;
        // React Query refetch resolves failures into query.error, which Pulse
        // renders with its retry action. A live update during this fetch queues
        // a second pass rather than cancelling or losing the in-flight result.
        void refreshLatest().then(() => {
          running = false;
          if (!disposed && dirty) schedule();
        });
      }, 1_000);
    };
    const stopCache = queryClient.getQueryCache().subscribe((event) => {
      if (event.type !== "updated" || event.action.type !== "success") return;
      const root = event.query.queryKey[0];
      if (
        root !== "channels" &&
        root !== "channel-messages" &&
        root !== "thread-replies"
      )
        return;
      const data = event.query.state.data;
      if (data === undefined || previous.get(event.query) === data) return;
      previous.set(event.query, data);
      schedule();
    });
    const stopFocus = subscribeAppFocus((focused) => {
      if (focused) schedule();
    });
    return () => {
      disposed = true;
      stopCache();
      stopFocus();
      if (timer !== undefined) clearTimeout(timer);
    };
  }, [enabled, queryClient, scope]);
}
