import { invoke as tauriInvoke } from "@tauri-apps/api/core";

export type GooseUpdateStatus =
  | {
      status: "up_to_date";
      installedVersion: string;
      latestVersion: string;
    }
  | {
      status: "update_available";
      installedVersion: string;
      latestVersion: string;
    };

export type RawGooseUpdateStatus =
  | {
      status: "up_to_date";
      installed_version: string;
      latest_version: string;
    }
  | {
      status: "update_available";
      installed_version: string;
      latest_version: string;
    };

export function fromRawGooseUpdateStatus(
  status: RawGooseUpdateStatus,
): GooseUpdateStatus {
  return {
    status: status.status,
    installedVersion: status.installed_version,
    latestVersion: status.latest_version,
  };
}

/** Refresh update availability only after a successful Goose setup command. */
export function shouldRefreshGooseUpdateStatus(
  runtimeId: string,
  succeeded: boolean,
): boolean {
  return runtimeId === "goose" && succeeded;
}

export async function checkGooseUpdateStatus(): Promise<GooseUpdateStatus> {
  const raw = await tauriInvoke<RawGooseUpdateStatus>(
    "check_goose_update_status",
  );
  return fromRawGooseUpdateStatus(raw);
}
