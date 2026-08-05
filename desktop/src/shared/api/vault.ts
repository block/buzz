import { invokeTauri } from "@/shared/api/tauri";
import type {
  VaultEntry,
  VaultFileContent,
  VaultInfo,
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
