/** A file or folder in the Documents vault tree. */
export type VaultEntry = {
  name: string;
  /** Absolute path, in the vault's own spelling (symlinks preserved). */
  path: string;
  isDirectory: boolean;
  /** `null` for files. */
  children: VaultEntry[] | null;
};

/** One result from a batch read. `content` is `null` when unreadable. */
export type VaultFileContent = {
  path: string;
  content: string | null;
};

/** The active vault. */
export type VaultInfo = {
  path: string;
  /** Basename, for display in the Documents header. */
  name: string;
};
