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

/** Result of an atomic note write. */
export type VaultWriteResult = {
  /**
   * Modification time of the file just written, in milliseconds since the
   * epoch, or 0 when the platform did not report one.
   */
  modifiedMs: number;
};

/** One entry from a `vault-file-modified` watcher event. */
export type VaultModifiedEntry = {
  path: string;
  modifiedMs: number;
};
