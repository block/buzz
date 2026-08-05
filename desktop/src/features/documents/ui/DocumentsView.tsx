import * as React from "react";
import { FolderOpen } from "lucide-react";

import {
  useVaultFileQuery,
  useVaultTreeQuery,
} from "@/features/documents/hooks";
import {
  ancestorFolderPaths,
  baseName,
  flattenVisibleRows,
  stripMarkdownExtension,
} from "@/features/documents/lib/treeModel";
import { useResizableDocumentsPanes } from "@/features/documents/useResizableDocumentsPanes";
import { useVaultLifecycle } from "@/features/documents/useVaultLifecycle";
import { DocumentPreview } from "@/features/documents/ui/DocumentPreview";
import { DocumentTreePane } from "@/features/documents/ui/DocumentTreePane";
import { VaultEmptyState } from "@/features/documents/ui/VaultEmptyState";
import { ChatHeader } from "@/features/chat/ui/ChatHeader";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Skeleton } from "@/shared/ui/skeleton";

export function DocumentsView() {
  const { activation, chooseVault } = useVaultLifecycle();
  const isReady = activation.status === "ready";
  const vaultRoot = isReady ? activation.path : null;

  const treeQuery = useVaultTreeQuery(vaultRoot);
  const { tree } = useResizableDocumentsPanes();

  const [expandedPaths, setExpandedPaths] = React.useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [activePath, setActivePath] = React.useState<string | null>(null);

  // A different vault is a different tree; drop selection and expansion rather
  // than carrying stale paths across. Adjusting during render (React's
  // "resetting state when a prop changes" pattern) rather than in an effect
  // avoids painting one frame of the new vault with the old selection.
  const [lastVaultRoot, setLastVaultRoot] = React.useState(vaultRoot);
  if (lastVaultRoot !== vaultRoot) {
    setLastVaultRoot(vaultRoot);
    setExpandedPaths(new Set());
    setActivePath(null);
  }

  const fileQuery = useVaultFileQuery(vaultRoot, activePath);

  const rows = React.useMemo(
    () => flattenVisibleRows(treeQuery.data ?? [], expandedPaths),
    [expandedPaths, treeQuery.data],
  );

  const handleToggleFolder = React.useCallback((path: string) => {
    setExpandedPaths((current) => {
      const next = new Set(current);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  }, []);

  const handleSelectFile = React.useCallback(
    (path: string) => {
      setActivePath(path);
      // Reveal the file's folders so the selection is visible after a jump.
      if (vaultRoot) {
        const ancestors = ancestorFolderPaths(vaultRoot, path);
        if (ancestors.length > 0) {
          setExpandedPaths((current) => new Set([...current, ...ancestors]));
        }
      }
    },
    [vaultRoot],
  );

  if (!isReady) {
    return (
      <>
        <ChatHeader mode="documents" title="Documents" />
        {activation.status === "activating" ? (
          <div className="flex min-h-0 flex-1 flex-col gap-3 p-6">
            <Skeleton className="h-6 w-48" />
            <Skeleton className="h-4 w-full max-w-xl" />
            <Skeleton className="h-4 w-2/3 max-w-xl" />
          </div>
        ) : (
          <VaultEmptyState
            errorMessage={
              activation.status === "error" ? activation.message : undefined
            }
            onChooseVault={() => void chooseVault()}
          />
        )}
      </>
    );
  }

  return (
    <>
      <ChatHeader
        actions={
          <Button
            data-testid="documents-change-vault"
            onClick={() => void chooseVault()}
            size="sm"
            type="button"
            variant="ghost"
          >
            <FolderOpen className="h-4 w-4" />
            Change folder
          </Button>
        }
        mode="documents"
        title={activation.name}
      />

      <div className="flex min-h-0 min-w-0 flex-1 overflow-hidden">
        <div
          className="flex min-h-0 shrink-0 flex-col overflow-y-auto border-r border-border/60"
          style={{ width: `${tree.widthPx}px` }}
        >
          {treeQuery.isLoading ? (
            <div className="space-y-2 p-3">
              <Skeleton className="h-3.5 w-28" />
              <Skeleton className="ml-3 h-3.5 w-32" />
              <Skeleton className="ml-3 h-3.5 w-24" />
            </div>
          ) : treeQuery.isError ? (
            <p className="px-3 py-4 text-sm text-destructive">
              {treeQuery.error instanceof Error
                ? treeQuery.error.message
                : "Could not read the vault folder."}
            </p>
          ) : (
            <DocumentTreePane
              activePath={activePath}
              onSelectFile={handleSelectFile}
              onToggleFolder={handleToggleFolder}
              rows={rows}
            />
          )}
        </div>

        <button
          aria-label="Resize the file tree"
          className={cn(
            "w-1 shrink-0 cursor-col-resize bg-transparent",
            "hover:bg-border focus-visible:bg-border",
          )}
          data-testid="documents-tree-resize-handle"
          onDoubleClick={tree.handleWidthReset}
          onPointerDown={tree.handleResizeStart}
          type="button"
        />

        <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto">
          {activePath === null ? (
            <p className="p-6 text-sm text-muted-foreground">
              Select a note to read it.
            </p>
          ) : fileQuery.isLoading ? (
            <div className="space-y-3 p-6">
              <Skeleton className="h-6 w-64" />
              <Skeleton className="h-4 w-full max-w-2xl" />
              <Skeleton className="h-4 w-4/5 max-w-2xl" />
            </div>
          ) : fileQuery.isError ? (
            <p className="p-6 text-sm text-destructive">
              {fileQuery.error instanceof Error
                ? fileQuery.error.message
                : "Could not read that note."}
            </p>
          ) : (
            <article className="p-6">
              <h1 className="mb-4 text-lg font-medium">
                {stripMarkdownExtension(baseName(activePath))}
              </h1>
              <DocumentPreview content={fileQuery.data ?? ""} />
            </article>
          )}
        </div>
      </div>
    </>
  );
}
