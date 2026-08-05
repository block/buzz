import { listen } from "@tauri-apps/api/event";

import { invokeTauri } from "@/shared/api/tauri";
import type {
  VaultEntry,
  VaultFileContent,
  VaultInfo,
  VaultModifiedEntry,
  VaultWriteResult,
} from "@/shared/api/vaultTypes";

type RawVaultEntry = {
  name: string;
  path: string;
  is_directory: boolean;
  children: RawVaultEntry[] | null;
};

type RawVaultFileContent = {
  path: string;
  content: string | null;
};

type RawVaultInfo = {
  path: string;
  name: string;
};

type RawVaultWriteResult = {
  modified_ms: number;
};

type RawVaultModifiedEntry = {
  path: string;
  modified_ms: number;
};

function fromRawVaultEntry(entry: RawVaultEntry): VaultEntry {
  return {
    name: entry.name,
    path: entry.path,
    isDirectory: entry.is_directory,
    children: entry.children ? entry.children.map(fromRawVaultEntry) : null,
  };
}

function fromRawVaultInfo(info: RawVaultInfo): VaultInfo {
  return { name: info.name, path: info.path };
}

/**
 * Show the native folder picker. Resolves to `null` when the user cancels.
 *
 * Selection alone grants nothing — call {@link setActiveVault} to activate it.
 */
export async function pickVaultFolder(): Promise<string | null> {
  const path = await invokeTauri<string | null>("pick_vault_folder");
  return path ?? null;
}

/** Activate a vault. Every other vault call operates within it. */
export async function setActiveVault(vaultPath: string): Promise<VaultInfo> {
  const info = await invokeTauri<RawVaultInfo>("set_active_vault", {
    vaultPath,
  });
  return fromRawVaultInfo(info);
}

export async function clearActiveVault(): Promise<void> {
  await invokeTauri<void>("clear_active_vault");
}

/** The vault the backend currently has active, for boot reconciliation. */
export async function getActiveVault(): Promise<VaultInfo | null> {
  const info = await invokeTauri<RawVaultInfo | null>("get_active_vault");
  return info ? fromRawVaultInfo(info) : null;
}

export async function listVaultFiles(): Promise<VaultEntry[]> {
  const entries = await invokeTauri<RawVaultEntry[]>("list_vault_files");
  return entries.map(fromRawVaultEntry);
}

export async function readVaultFile(path: string): Promise<string> {
  return await invokeTauri<string>("read_vault_file", { path });
}

/** Batch read — one IPC round trip for the whole note-index corpus. */
export async function readVaultFiles(
  paths: string[],
): Promise<VaultFileContent[]> {
  const results = await invokeTauri<RawVaultFileContent[]>("read_vault_files", {
    paths,
  });
  return results.map((result) => ({
    path: result.path,
    content: result.content,
  }));
}

export async function vaultEntryExists(path: string): Promise<boolean> {
  return await invokeTauri<boolean>("vault_entry_exists", { path });
}

/**
 * Writes a note atomically.
 *
 * The returned mtime is what lets the watcher tell our own save apart from a
 * genuine external edit — see `useVaultWatcher`.
 */
export async function writeVaultFile(
  path: string,
  content: string,
): Promise<VaultWriteResult> {
  const result = await invokeTauri<RawVaultWriteResult>("write_vault_file", {
    content,
    path,
  });
  return { modifiedMs: result.modified_ms };
}

export async function createVaultFile(path: string): Promise<void> {
  await invokeTauri<void>("create_vault_file", { path });
}

export async function createVaultFolder(path: string): Promise<void> {
  await invokeTauri<void>("create_vault_folder", { path });
}

/** Renames or moves an entry. Both endpoints must be inside the vault. */
export async function renameVaultEntry(
  oldPath: string,
  newPath: string,
): Promise<void> {
  await invokeTauri<void>("rename_vault_entry", { newPath, oldPath });
}

export async function deleteVaultEntry(path: string): Promise<void> {
  await invokeTauri<void>("delete_vault_entry", { path });
}

export async function startVaultWatch(): Promise<void> {
  await invokeTauri<void>("start_vault_watch");
}

export async function stopVaultWatch(): Promise<void> {
  await invokeTauri<void>("stop_vault_watch");
}

/** Subscribes to content changes. Resolves to an unsubscribe function. */
export async function onVaultFileModified(
  callback: (entries: VaultModifiedEntry[]) => void,
): Promise<() => void> {
  const unlisten = await listen<RawVaultModifiedEntry[]>(
    "vault-file-modified",
    (event) => {
      callback(
        event.payload.map((entry) => ({
          modifiedMs: entry.modified_ms,
          path: entry.path,
        })),
      );
    },
  );
  return () => unlisten();
}

/** Subscribes to create/delete/rename events. */
export async function onVaultFilesChanged(
  callback: () => void,
): Promise<() => void> {
  const unlisten = await listen("vault-files-changed", () => callback());
  return () => unlisten();
}
