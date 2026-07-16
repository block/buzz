import * as React from "react";
import { listen } from "@tauri-apps/api/event";

/** Mirror of the Rust MeshDownloadProgress payload (mesh_llm/progress.rs). */
export type MeshDownloadProgress = {
  label: string;
  file: string | null;
  downloadedBytes: number | null;
  totalBytes: number | null;
  /** preparing | downloading | done */
  status: "preparing" | "downloading" | "done";
  done: boolean;
};

/**
 * Live model-download progress from the backend's mesh output sink.
 * Returns the latest event while a download is active, `null` when idle or
 * after completion. `reset()` clears a lingering final event (call it when a
 * start action settles).
 */
export function useMeshDownloadProgress(): {
  progress: MeshDownloadProgress | null;
  reset: () => void;
} {
  const [progress, setProgress] = React.useState<MeshDownloadProgress | null>(
    null,
  );

  React.useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    (async () => {
      try {
        const stop = await listen<MeshDownloadProgress>(
          "mesh-download-progress",
          (event) => {
            if (cancelled) return;
            setProgress(event.payload.done ? null : event.payload);
          },
        );
        if (cancelled) {
          stop();
        } else {
          unlisten = stop;
        }
      } catch {
        // Event system unavailable (web/e2e) — progress just doesn't render.
      }
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const reset = React.useCallback(() => setProgress(null), []);
  return { progress, reset };
}

/** "2.1 GB of 5.0 GB" style formatting; tolerates unknown totals. */
export function formatDownloadBytes(p: MeshDownloadProgress): string {
  const gb = (n: number) => `${(n / 1e9).toFixed(1)} GB`;
  if (p.downloadedBytes == null) return "";
  if (p.totalBytes == null || p.totalBytes === 0) return gb(p.downloadedBytes);
  return `${gb(p.downloadedBytes)} of ${gb(p.totalBytes)}`;
}

/** 0..100 or null when the total is unknown. */
export function downloadPercent(p: MeshDownloadProgress): number | null {
  if (p.downloadedBytes == null || p.totalBytes == null || p.totalBytes === 0) {
    return null;
  }
  return Math.min(100, Math.round((p.downloadedBytes / p.totalBytes) * 100));
}
