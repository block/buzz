import type {
  InstallerDownloadEvent,
  InstallerDownloadOutcome,
  InstallerFailureReason,
} from "@/shared/api/newAppInstallerDownload";

type MockChannel = { onmessage?: (event: InstallerDownloadEvent) => void };

type ActiveDownload = {
  channel: MockChannel | undefined;
  fileName: string;
  settle: (outcome: InstallerDownloadOutcome) => void;
};

declare global {
  interface Window {
    /**
     * Drives the mock `download_new_app_installer` command, which stays
     * pending until a spec finishes or fails it, or the app cancels it.
     */
    __BUZZ_E2E_INSTALLER_DOWNLOAD__?: {
      progress: (receivedBytes: number, totalBytes: number | null) => void;
      finish: () => void;
      fail: (reason: InstallerFailureReason) => void;
      isActive: () => boolean;
    };
  }
}

const MOCK_DOWNLOADS_FOLDER = "/Users/e2e/Downloads";

// Like the backend, the mock runs one download at a time.
let active: ActiveDownload | null = null;

export function handleMockInstallerDownload(
  payload: unknown,
): Promise<InstallerDownloadOutcome> {
  if (active) {
    return Promise.resolve({ status: "failed", reason: "busy" });
  }
  const { url, onEvent } = payload as { url: string; onEvent?: MockChannel };
  const fileName = new URL(url).pathname.split("/").at(-1) ?? "";
  return new Promise((resolve) => {
    active = {
      channel: onEvent,
      fileName,
      settle: (outcome) => {
        active = null;
        resolve(outcome);
      },
    };
    onEvent?.onmessage?.({ kind: "started", fileName });
  });
}

export function handleMockInstallerCancel(): null {
  active?.settle({ status: "cancelled" });
  return null;
}

export function installMockInstallerDownloadControls() {
  active = null;
  window.__BUZZ_E2E_INSTALLER_DOWNLOAD__ = {
    progress: (receivedBytes, totalBytes) =>
      active?.channel?.onmessage?.({
        kind: "progress",
        receivedBytes,
        totalBytes,
      }),
    finish: () =>
      active?.settle({
        status: "saved",
        fileName: active.fileName,
        folder: MOCK_DOWNLOADS_FOLDER,
      }),
    fail: (reason) => active?.settle({ status: "failed", reason }),
    isActive: () => active !== null,
  };
}
