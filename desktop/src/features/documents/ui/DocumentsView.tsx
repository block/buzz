import * as React from "react";
import { FolderOpen, PanelRight } from "lucide-react";

import {
  useVaultContentsQuery,
  useVaultTreeQuery,
} from "@/features/documents/hooks";
import { useDocumentSession } from "@/features/documents/useDocumentSession";
import {
  ancestorFolderPaths,
  flattenVisibleRows,
} from "@/features/documents/lib/treeModel";
import { useResizableDocumentsPanes } from "@/features/documents/useResizableDocumentsPanes";
import { useVaultLifecycle } from "@/features/documents/useVaultLifecycle";
import { DocumentEditorPane } from "@/features/documents/ui/DocumentEditorPane";
import { DocumentTabBar } from "@/features/documents/ui/DocumentTabBar";
import { DocumentTreePane } from "@/features/documents/ui/DocumentTreePane";
import { DocumentBacklinksPanel } from "@/features/documents/ui/DocumentBacklinksPanel";
import { DocumentOutlinePanel } from "@/features/documents/ui/DocumentOutlinePanel";
import type { OutlineHeading } from "@/features/documents/lib/obsidianSyntax";
import {
  readSessionSnapshot,
  writeSessionSnapshot,
} from "@/features/documents/lib/documentSession";
import { getBacklinks } from "@/features/documents/lib/backlinks";
import { buildNoteIndex } from "@/features/documents/lib/noteIndex";
import { collectFilePaths } from "@/features/documents/lib/treeModel";
import { DocumentDeleteDialog } from "@/features/documents/ui/DocumentTreeContextMenu";
import {
  DocumentNamePromptDialog,
  type NamePrompt,
} from "@/features/documents/ui/DocumentNamePromptDialog";
import { useVaultMutations } from "@/features/documents/useVaultMutations";
import { baseName, parentOf } from "@/features/documents/lib/treeModel";
import type { VaultEntry } from "@/shared/api/vaultTypes";
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
  const { rail, tree } = useResizableDocumentsPanes();

  const [expandedPaths, setExpandedPaths] = React.useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const session = useDocumentSession(vaultRoot);
  const activePath = session.state.activePath;

  // A different vault is a different tree; drop selection and expansion rather
  // than carrying stale paths across. Adjusting during render (React's
  // "resetting state when a prop changes" pattern) rather than in an effect
  // avoids painting one frame of the new vault with the old selection.
  const [lastVaultRoot, setLastVaultRoot] = React.useState(vaultRoot);
  if (lastVaultRoot !== vaultRoot) {
    setLastVaultRoot(vaultRoot);
    setExpandedPaths(new Set());
  }

  const rows = React.useMemo(
    () => flattenVisibleRows(treeQuery.data ?? [], expandedPaths),
    [expandedPaths, treeQuery.data],
  );

  // The corpus every wikilink and backlink is resolved against.
  const contentsQuery = useVaultContentsQuery(vaultRoot, treeQuery.data);

  const noteIndex = React.useMemo(() => {
    if (!vaultRoot || !treeQuery.data) return null;
    return buildNoteIndex(vaultRoot, collectFilePaths(treeQuery.data));
  }, [treeQuery.data, vaultRoot]);

  const backlinks = React.useMemo(() => {
    if (!activePath || !contentsQuery.data) {
      return { linked: [], unlinked: [] };
    }
    return getBacklinks({
      contents: contentsQuery.data,
      index: noteIndex,
      targetPath: activePath,
    });
  }, [activePath, contentsQuery.data, noteIndex]);

  // Restore the previous session once per vault. Content is never persisted —
  // each note is re-read from disk, so a file changed since last run wins.
  const restoredForRef = React.useRef<string | null>(null);
  React.useEffect(() => {
    if (!vaultRoot || restoredForRef.current === vaultRoot) return;
    restoredForRef.current = vaultRoot;

    const snapshot = readSessionSnapshot(vaultRoot);
    if (!snapshot) return;
    setExpandedPaths(new Set(snapshot.expandedPaths));
    void session.restoreFiles(snapshot.openPaths, snapshot.activePath);
  }, [session.restoreFiles, vaultRoot]);

  // Persist after the restore has run, so an empty initial state cannot
  // clobber a saved session.
  React.useEffect(() => {
    if (!vaultRoot || restoredForRef.current !== vaultRoot) return;
    writeSessionSnapshot({
      activePath: session.state.activePath,
      expandedPaths: [...expandedPaths],
      openPaths: session.state.tabs.map((tab) => tab.path),
      vaultPath: vaultRoot,
    });
  }, [expandedPaths, session.state, vaultRoot]);

  const [headings, setHeadings] = React.useState<OutlineHeading[]>([]);
  const [activeHeading, setActiveHeading] = React.useState(-1);
  const [railOpen, setRailOpen] = React.useState(() => {
    try {
      return window.localStorage.getItem("buzz.documents.railOpen") !== "0";
    } catch {
      return true;
    }
  });
  const toggleRail = React.useCallback(() => {
    setRailOpen((current) => {
      const next = !current;
      try {
        window.localStorage.setItem(
          "buzz.documents.railOpen",
          next ? "1" : "0",
        );
      } catch {
        // Losing the preference is not worth failing the interaction.
      }
      return next;
    });
  }, []);
  // Set by the live editor once it mounts; the outline lives beside the editor
  // rather than inside it, so it cannot reach the view directly.
  const scrollToHeadingRef = React.useRef<((position: number) => void) | null>(
    null,
  );
  const registerScroll = React.useCallback(
    (scroll: ((position: number) => void) | null) => {
      scrollToHeadingRef.current = scroll;
    },
    [],
  );

  const handleWikilinkClick = React.useCallback(
    ({ exists, path }: { exists: boolean; path: string | null }) => {
      // Broken links have nowhere to go yet; creating on click would litter the
      // vault with empty notes from stray clicks.
      if (!exists || !path) return;
      void session.openFile(path);
    },
    [session.openFile],
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

  const mutations = useVaultMutations(vaultRoot);
  const [namePrompt, setNamePrompt] = React.useState<NamePrompt | null>(null);
  const [pendingDelete, setPendingDelete] = React.useState<VaultEntry | null>(
    null,
  );

  /** New entries go inside a folder, or beside a file. */
  const containerFor = React.useCallback(
    (entry: VaultEntry) =>
      entry.isDirectory ? entry.path : parentOf(entry.path),
    [],
  );

  const handleNameSubmit = React.useCallback(
    async (value: string) => {
      const prompt = namePrompt;
      setNamePrompt(null);
      if (!prompt) return;

      if (prompt.kind === "note") {
        const created = await mutations.createNote(prompt.contextPath, value);
        // Open the new note so the user can start typing straight away.
        if (created) void session.openFile(created);
        return;
      }
      if (prompt.kind === "folder") {
        await mutations.createFolder(prompt.contextPath, value);
        return;
      }

      const isDirectory = !prompt.contextPath.match(/\.(?:md|markdown)$/i);
      const renamed = await mutations.rename(
        prompt.contextPath,
        value,
        isDirectory,
      );
      // Keep any open buffer pointed at the file rather than a dead path.
      if (renamed) session.notePathRenamed(prompt.contextPath, renamed);
    },
    [mutations, namePrompt, session],
  );

  const handleSelectFile = React.useCallback(
    (path: string) => {
      void session.openFile(path);
      // Reveal the file's folders so the selection is visible after a jump.
      if (vaultRoot) {
        const ancestors = ancestorFolderPaths(vaultRoot, path);
        if (ancestors.length > 0) {
          setExpandedPaths((current) => new Set([...current, ...ancestors]));
        }
      }
    },
    [session.openFile, vaultRoot],
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
          <>
            <Button
              aria-pressed={railOpen}
              data-testid="documents-toggle-rail"
              onClick={toggleRail}
              size="sm"
              title={
                railOpen
                  ? "Hide outline and backlinks"
                  : "Show outline and backlinks"
              }
              type="button"
              variant="ghost"
            >
              <PanelRight className="h-4 w-4" />
            </Button>
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
          </>
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
              onCreateFolderIn={(entry) =>
                setNamePrompt({
                  contextPath: containerFor(entry),
                  initialValue: "",
                  kind: "folder",
                })
              }
              onCreateNoteIn={(entry) =>
                setNamePrompt({
                  contextPath: containerFor(entry),
                  initialValue: "",
                  kind: "note",
                })
              }
              onDelete={setPendingDelete}
              onRename={(entry) =>
                setNamePrompt({
                  contextPath: entry.path,
                  initialValue: baseName(entry.path),
                  kind: "rename",
                })
              }
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

        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          <DocumentTabBar
            activePath={activePath}
            onActivate={session.activateFile}
            onClose={(path) => void session.closeFile(path)}
            tabs={session.state.tabs}
          />

          {session.activeTab ? (
            <DocumentEditorPane
              hasExternalChange={session.externalChanges.has(
                session.activeTab.path,
              )}
              headings={headings}
              noteIndex={noteIndex}
              onActiveHeadingChange={setActiveHeading}
              onHeadingsChange={setHeadings}
              onRegisterScroll={registerScroll}
              onWikilinkClick={handleWikilinkClick}
              onChange={(markdown) => {
                if (session.activeTab) {
                  session.updateTabContent(session.activeTab.path, markdown);
                }
              }}
              onKeepMine={() => {
                if (session.activeTab) {
                  session.keepLocalVersion(session.activeTab.path);
                }
              }}
              onReload={() => {
                if (session.activeTab) {
                  void session.reloadFile(session.activeTab.path);
                }
              }}
              onSave={() => {
                if (session.activeTab) {
                  void session.saveTab(session.activeTab.path);
                }
              }}
              onSetViewMode={(mode) => {
                if (session.activeTab) {
                  session.setViewMode(session.activeTab.path, mode);
                }
              }}
              tab={session.activeTab}
            />
          ) : (
            <p className="p-6 text-sm text-muted-foreground">
              Select a note to open it.
            </p>
          )}
        </div>

        {session.activeTab && railOpen ? (
          <>
            <button
              aria-label="Resize the backlinks rail"
              className={cn(
                "w-1 shrink-0 cursor-col-resize bg-transparent",
                "hover:bg-border focus-visible:bg-border",
              )}
              data-testid="documents-rail-resize-handle"
              onDoubleClick={rail.handleWidthReset}
              onPointerDown={rail.handleResizeStart}
              type="button"
            />
            <div
              className="flex min-h-0 shrink-0 flex-col border-l border-border/60"
              style={{ width: `${rail.widthPx}px` }}
            >
              <div className="min-h-0 flex-1 overflow-y-auto">
                <DocumentOutlinePanel
                  activeIndex={activeHeading}
                  headings={headings}
                  onSelect={(heading) =>
                    scrollToHeadingRef.current?.(heading.position)
                  }
                />
                <DocumentBacklinksPanel
                  backlinks={backlinks}
                  onOpen={(path) => void session.openFile(path)}
                />
              </div>
            </div>
          </>
        ) : null}
      </div>

      <DocumentNamePromptDialog
        onCancel={() => setNamePrompt(null)}
        onSubmit={(value) => void handleNameSubmit(value)}
        prompt={namePrompt}
      />
      <DocumentDeleteDialog
        entry={pendingDelete}
        onCancel={() => setPendingDelete(null)}
        onConfirm={() => {
          const target = pendingDelete;
          setPendingDelete(null);
          if (target) void session.deleteEntry(target.path);
        }}
      />
    </>
  );
}
