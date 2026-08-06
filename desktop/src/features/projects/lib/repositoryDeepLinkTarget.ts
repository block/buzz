export function effectiveProjectRepoSource(
  selectedSource: "remote" | "local",
  repositoryPath: string | undefined,
): "remote" | "local" {
  return repositoryPath ? "remote" : selectedSource;
}

export type RepositoryDeepLinkTarget<T extends { path: string }> =
  | { kind: "file"; file: T; parentPath: string }
  | { kind: "directory"; path: string }
  | { kind: "missing" };

export function resolveRepositoryDeepLinkTarget<T extends { path: string }>(
  files: readonly T[],
  targetPath: string,
): RepositoryDeepLinkTarget<T> {
  const file = files.find((candidate) => candidate.path === targetPath);
  if (file) {
    const slash = targetPath.lastIndexOf("/");
    return {
      kind: "file",
      file,
      parentPath: slash >= 0 ? targetPath.slice(0, slash) : "",
    };
  }
  const directoryPrefix = `${targetPath}/`;
  if (files.some((candidate) => candidate.path.startsWith(directoryPrefix))) {
    return { kind: "directory", path: targetPath };
  }
  return { kind: "missing" };
}

export function projectTabForRepositoryTarget(
  targetPath: string | undefined,
): "files" | "overview" {
  return targetPath ? "files" : "overview";
}

export type RepositoryTargetAttempt = {
  key: string;
  outcome: "error" | "resolved";
};

export function shouldResolveRepositoryTarget(input: {
  attempt: RepositoryTargetAttempt | null;
  hasError: boolean;
  isLoading: boolean;
  resolutionKey: string | null;
}): boolean {
  if (!input.resolutionKey || input.isLoading) return false;
  if (input.attempt?.key !== input.resolutionKey) return true;
  return input.attempt.outcome === "error" && !input.hasError;
}

export function repositoryTargetResultKey(
  targetKey: string,
  filesKey: string,
): string {
  return `${targetKey}\0${filesKey}`;
}

export function repositoryRootToastAction(onClick: () => void): {
  label: string;
  onClick: () => void;
} {
  return { label: "Open repository root", onClick };
}
