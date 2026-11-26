import * as React from "react";

import type { ProjectRepoFile } from "@/features/projects/hooks";

export type RepositoryFilesContext = {
  kind: "file" | "folder";
  onBack?: () => void;
  path: string;
};

export function useRepositoryFilesNavigation({
  files,
  initialPath,
  onContextChange,
  onPathChange,
  pageSize,
}: {
  files: ProjectRepoFile[];
  initialPath?: string;
  onContextChange?: (context: RepositoryFilesContext) => void;
  onPathChange?: (path: string | null) => void;
  pageSize: number;
}) {
  const [currentPath, setCurrentPath] = React.useState("");
  const [selectedFile, setSelectedFile] =
    React.useState<ProjectRepoFile | null>(null);
  const [visibleEntryCount, setVisibleEntryCount] = React.useState(pageSize);
  const openPath = React.useCallback(
    (path: string) => {
      setCurrentPath(path);
      setSelectedFile(null);
      setVisibleEntryCount(pageSize);
      onPathChange?.(path || null);
    },
    [onPathChange, pageSize],
  );
  const selectFile = React.useCallback(
    (file: ProjectRepoFile | null) => {
      setSelectedFile(file);
      onPathChange?.(file?.path ?? (currentPath || null));
    },
    [currentPath, onPathChange],
  );
  const contextBack = React.useMemo(() => {
    if (selectedFile) return () => selectFile(null);
    if (!currentPath) return undefined;
    return () => {
      const segments = currentPath.split("/");
      openPath(segments.slice(0, -1).join("/"));
    };
  }, [currentPath, openPath, selectedFile, selectFile]);

  React.useEffect(() => {
    onContextChange?.({
      kind: selectedFile ? "file" : "folder",
      onBack: contextBack,
      path: selectedFile?.path ?? currentPath,
    });
  }, [contextBack, currentPath, onContextChange, selectedFile]);

  const filesKey = React.useMemo(
    () => files.map((file) => file.path).join("\0"),
    [files],
  );
  const initialFile = React.useMemo(
    () =>
      initialPath
        ? (files.find((file) => file.path === initialPath) ?? null)
        : null,
    [files, initialPath],
  );
  React.useEffect(() => {
    if (!filesKey) return;
    const initialDirectory = initialFile
      ? initialFile.path.split("/").slice(0, -1).join("/")
      : (initialPath ?? "");
    setCurrentPath(initialDirectory);
    setSelectedFile(initialFile);
    setVisibleEntryCount(pageSize);
  }, [filesKey, initialFile, initialPath, pageSize]);

  return {
    currentPath,
    openPath,
    selectedFile,
    setSelectedFile: selectFile,
    setVisibleEntryCount,
    visibleEntryCount,
  };
}
