import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type PendingDesktopControlImport = {
  requestId: string;
  fileName: string;
  fileBytes: number[];
};

const REQUEST_AVAILABLE_EVENT = "desktop-control-request-available";

export async function cancelPendingDesktopControlImport(
  requestId: string,
): Promise<boolean> {
  return invoke<boolean>("cancel_pending_desktop_control_import", {
    requestId,
  });
}

/**
 * Drain owner-reviewed imports submitted through the local Desktop control
 * socket. Rust retains one request until this router-aware listener consumes it,
 * so a CLI request received during frontend startup is not lost.
 */
export async function listenForDesktopControlImports(
  onOpen: (request: PendingDesktopControlImport) => void,
): Promise<UnlistenFn> {
  let drainRunning = false;
  let drainRequested = false;
  const drain = () => {
    drainRequested = true;
    if (drainRunning) return;
    drainRunning = true;
    void (async () => {
      try {
        while (drainRequested) {
          drainRequested = false;
          const pending = await invoke<PendingDesktopControlImport | null>(
            "take_pending_desktop_control_import",
          );
          if (pending) onOpen(pending);
        }
      } catch (error: unknown) {
        console.warn("Failed to drain Desktop control imports", error);
      } finally {
        drainRunning = false;
        if (drainRequested) drain();
      }
    })();
  };
  const unlisten = await listen(REQUEST_AVAILABLE_EVENT, drain);
  drain();
  return unlisten;
}
