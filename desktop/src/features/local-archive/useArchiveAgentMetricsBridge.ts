import * as React from "react";
import { listen } from "@tauri-apps/api/event";

import { notifyAgentMetricsChanged } from "@/shared/api/tauriArchive";

/**
 * Bridges the backend's `archive-agent-metrics-changed` event to the existing
 * in-process notifier.
 *
 * The archive batch that persists agent-metric rows now runs entirely in Rust,
 * so the notifier's archive-side producer became a Tauri event. Consumers
 * (`useAgentUsageSeries`) keep subscribing to `onAgentMetricsChanged` exactly
 * as before — the source moved, the contract did not.
 */
export function useArchiveAgentMetricsBridge(): void {
  React.useEffect(() => {
    const unlisten = listen("archive-agent-metrics-changed", () => {
      notifyAgentMetricsChanged();
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);
}
