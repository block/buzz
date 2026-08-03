export type ProjectFileTreeSummary = {
  countLabel: string;
  truncationNotice: string | null;
};

/** Builds honest file-count copy for complete and capped repository snapshots. */
export function projectFileTreeSummary(
  loadedFileCount: number,
  reportedTotalFileCount: number,
): ProjectFileTreeSummary {
  const totalFileCount = Math.max(loadedFileCount, reportedTotalFileCount);
  if (loadedFileCount >= totalFileCount) {
    return {
      countLabel: `${loadedFileCount} file${loadedFileCount === 1 ? "" : "s"}`,
      truncationNotice: null,
    };
  }

  return {
    countLabel: `${loadedFileCount} of ${totalFileCount} files`,
    truncationNotice: `Showing the first ${loadedFileCount} of ${totalFileCount} files. Some files and folders are not included.`,
  };
}
