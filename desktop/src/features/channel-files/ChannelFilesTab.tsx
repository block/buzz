import { useCallback, useMemo, useRef, useState } from "react";
import {
  Search,
  ArrowUpDown,
  FolderPlus,
  Folder,
  ChevronRight,
  ChevronDown,
  Trash2,
  X,
  Undo2,
  FolderInput,
} from "lucide-react";
import { FileRow, FileRowSkeleton } from "./FileCard";
import { type FileFolder } from "./useFileFolders";
import {
  categorizeFile,
  sortFiles,
  type ChannelFile,
  type FileCategory,
  type FileSort,
} from "./useChannelFiles";
import { Button } from "@/shared/ui/button";

const CATEGORY_TABS: { value: FileCategory; label: string }[] = [
  { value: "all", label: "All" },
  { value: "image", label: "Images" },
  { value: "video", label: "Videos" },
  { value: "document", label: "Documents" },
  { value: "other", label: "Other" },
];

const SORT_OPTIONS: { value: FileSort; label: string }[] = [
  { value: "newest", label: "Newest" },
  { value: "oldest", label: "Oldest" },
  { value: "name", label: "Name" },
  { value: "size", label: "Size" },
];

export type ChannelFilesTabProps = {
  files: ChannelFile[];
  isLoading: boolean;
  senderNames?: Map<string, string>;
  senderAvatarUrls?: Map<string, string | null>;
  onJumpToMessage?: (eventId: string) => void;
  folders?: FileFolder[];
  foldersLoading?: boolean;
  fileFolderMap?: Map<string, string>;
  onCreateFolder?: (name: string) => Promise<unknown>;
  onDeleteFolder?: (folder: FileFolder) => Promise<unknown>;
  onRenameFolder?: (folder: FileFolder, name: string) => Promise<unknown>;
  onAddFileToFolder?: (folder: FileFolder, eventId: string) => Promise<unknown>;
  onAddFilesToFolder?: (folder: FileFolder, eventIds: string[]) => Promise<unknown>;
  onRemoveFileFromFolder?: (folder: FileFolder, eventId: string) => Promise<unknown>;
  onSetFolderParent?: (folder: FileFolder, parentDTag?: string) => Promise<unknown>;
};

export function ChannelFilesTab({
  files,
  isLoading,
  senderNames,
  senderAvatarUrls,
  onJumpToMessage,
  folders = [],
  fileFolderMap,
  onCreateFolder,
  onDeleteFolder,
  onAddFileToFolder,
  onAddFilesToFolder,
  onRemoveFileFromFolder,
  onSetFolderParent,
}: ChannelFilesTabProps) {
  const [category, setCategory] = useState<FileCategory>("all");
  const [searchQuery, setSearchQuery] = useState("");
  const [sort, setSort] = useState<FileSort>("newest");
  const [expandedFolders, setExpandedFolders] = useState<Set<string>>(
    () => new Set(),
  );
  const [isCreatingFolder, setIsCreatingFolder] = useState(false);
  const [newFolderName, setNewFolderName] = useState("");
  const [dragOverFolder, setDragOverFolder] = useState<string | null>(null);

  // Selection
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const lastClickedRef = useRef<string | null>(null);

  const filtered = useMemo(() => {
    let result = files;
    if (category !== "all") {
      result = result.filter((f) => categorizeFile(f.mimeType) === category);
    }
    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase().trim();
      result = result.filter(
        (f) =>
          (f.filename ?? "").toLowerCase().includes(q) ||
          (f.caption ?? "").toLowerCase().includes(q),
      );
    }
    return sortFiles(result, sort);
  }, [files, category, searchQuery, sort]);

  const counts = useMemo(() => {
    const c: Record<FileCategory, number> = {
      all: files.length,
      image: 0,
      video: 0,
      document: 0,
      other: 0,
    };
    for (const f of files) {
      c[categorizeFile(f.mimeType)]++;
    }
    return c;
  }, [files]);

  const filesByFolder = useMemo(() => {
    const map = new Map<string, ChannelFile[]>();
    if (!fileFolderMap) return map;
    for (const file of filtered) {
      const dTag = fileFolderMap.get(file.eventId);
      if (dTag) {
        const list = map.get(dTag) ?? [];
        list.push(file);
        map.set(dTag, list);
      }
    }
    return map;
  }, [filtered, fileFolderMap]);

  const unfiledFiles = useMemo(
    () =>
      fileFolderMap
        ? filtered.filter((f) => !fileFolderMap.has(f.eventId))
        : filtered,
    [filtered, fileFolderMap],
  );

  const allVisibleIds = useMemo(() => {
    const ids: string[] = [];
    for (const f of unfiledFiles) ids.push(f.eventId);
    for (const dTag of expandedFolders) {
      for (const f of filesByFolder.get(dTag) ?? []) ids.push(f.eventId);
    }
    return ids;
  }, [unfiledFiles, expandedFolders, filesByFolder]);

  // Determine which selected files are inside a folder (for bulk remove)
  const selectedInFolder = useMemo(() => {
    if (!fileFolderMap || selectedIds.size === 0) return null;
    let commonDTag: string | null = null;
    for (const id of selectedIds) {
      const dTag = fileFolderMap.get(id);
      if (dTag) {
        if (commonDTag === null) commonDTag = dTag;
        else if (commonDTag !== dTag) return null;
      } else {
        return null;
      }
    }
    return commonDTag;
  }, [fileFolderMap, selectedIds]);

  // Tree structure: parent → children
  const folderTree = useMemo(() => {
    const children = new Map<string | "root", FileFolder[]>();
    for (const f of folders) {
      const parent = f.parentDTag ?? "root";
      const list = children.get(parent) ?? [];
      list.push(f);
      children.set(parent, list);
    }
    return children;
  }, [folders]);

  // Flat list with depth for rendering — only includes children of expanded folders
  const flatFolders = useMemo(() => {
    const result: { folder: FileFolder; depth: number }[] = [];
    const root = folderTree.get("root") ?? [];
    function walk(parentDTag: string | undefined, depth: number) {
      const children = parentDTag
        ? folderTree.get(parentDTag) ?? []
        : root;
      for (const f of children) {
        result.push({ folder: f, depth });
        // Only recurse into children if this folder is expanded
        if (expandedFolders.has(f.dTag)) {
          walk(f.dTag, depth + 1);
        }
      }
    }
    walk(undefined, 0);
    return result;
  }, [folderTree, expandedFolders]);

  function toggleFolder(dTag: string) {
    setExpandedFolders((prev) => {
      const next = new Set(prev);
      if (next.has(dTag)) next.delete(dTag);
      else next.add(dTag);
      return next;
    });
  }

  async function handleCreateFolder() {
    if (!newFolderName.trim() || !onCreateFolder) return;
    await onCreateFolder(newFolderName.trim());
    setNewFolderName("");
    setIsCreatingFolder(false);
  }

  const handleDragStart = useCallback(
    (e: React.DragEvent, eventId: string) => {
      e.dataTransfer.setData("text/plain", eventId);
      e.dataTransfer.effectAllowed = "move";
    },
    [],
  );

  const handleFolderDragOver = useCallback(
    (e: React.DragEvent, dTag: string) => {
      e.preventDefault();
      e.dataTransfer.dropEffect = "move";
      setDragOverFolder(dTag);
    },
    [],
  );

  const handleFolderDragLeave = useCallback(() => {
    setDragOverFolder(null);
  }, []);

  const handleFolderDrop = useCallback(
    async (e: React.DragEvent, folder: FileFolder) => {
      e.preventDefault();
      setDragOverFolder(null);
      // Check if a folder is being dragged (nesting) vs a file
      const folderDTag = e.dataTransfer.getData("application/x-folder");
      if (folderDTag) {
        // Don't nest a folder into itself or its children
        if (folderDTag === folder.dTag) return;
        onSetFolderParent?.(folders.find((f) => f.dTag === folderDTag)!, folder.dTag);
        return;
      }
      const eventId = e.dataTransfer.getData("text/plain");
      if (!eventId || !onAddFileToFolder) return;
      if (fileFolderMap?.get(eventId) === folder.dTag) return;
      await onAddFileToFolder(folder, eventId);
    },
    [fileFolderMap, onAddFileToFolder, onSetFolderParent, folders],
  );

  // ── Selection ────────────────────────────────────────────────────

  function handleToggleSelect(eventId: string, e?: React.MouseEvent) {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      const shift = e?.shiftKey ?? false;

      if (shift && lastClickedRef.current) {
        const lastIdx = allVisibleIds.indexOf(lastClickedRef.current);
        const thisIdx = allVisibleIds.indexOf(eventId);
        if (lastIdx !== -1 && thisIdx !== -1) {
          const [start, end] = lastIdx < thisIdx ? [lastIdx, thisIdx] : [thisIdx, lastIdx];
          for (let i = start; i <= end; i++) {
            next.add(allVisibleIds[i]);
          }
        }
      } else if (next.has(eventId)) {
        next.delete(eventId);
      } else {
        next.add(eventId);
      }

      lastClickedRef.current = eventId;
      return next;
    });
  }

  function selectAll() {
    setSelectedIds(new Set(allVisibleIds));
  }

  async function handleBulkMoveToFolder(dTag: string) {
    if (!onAddFilesToFolder) return;
    const folder = folders.find((f) => f.dTag === dTag);
    if (!folder) return;
    const ids = Array.from(selectedIds);
    await onAddFilesToFolder(folder, ids);
    toast(`Moved ${ids.length} file${ids.length !== 1 ? "s" : ""} to ${folder.name}`);
    setSelectedIds(new Set());
  }

  async function handleBulkRemoveFromFolder() {
    if (!onRemoveFileFromFolder || !selectedInFolder) return;
    const folder = folders.find((f) => f.dTag === selectedInFolder);
    if (!folder) return;
    const count = selectedIds.size;
    for (const eventId of selectedIds) {
      await onRemoveFileFromFolder(folder, eventId);
    }
    toast(`Removed ${count} file${count !== 1 ? "s" : ""} from ${folder.name}`);
    setSelectedIds(new Set());
  }

  function toast(msg: string) {
    import("sonner").then(({ toast: t }) => t.success(msg)).catch(() => {});
  }

  const selectedCount = selectedIds.size;

  function renderFileRow(file: ChannelFile) {
    return (
      <FileRow
        file={file}
        key={file.key}
        onDragStart={handleDragStart}
        onJumpToMessage={onJumpToMessage}
        onToggleSelect={(id) => handleToggleSelect(id)}
        selected={selectedIds.has(file.eventId)}
        selecting={true}
        senderAvatarUrl={senderAvatarUrls?.get(file.pubkey) ?? null}
        senderName={senderNames?.get(file.pubkey)}
      />
    );
  }

  if (isLoading) {
    return (
      <div className="flex-1 overflow-y-auto p-4">
        <div className="divide-y divide-border">
          {Array.from({ length: 8 }).map((_, i) => (
            <FileRowSkeleton key={i} />
          ))}
        </div>
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* Toolbar */}
      <div className="shrink-0 space-y-2 border-b border-border px-4 pb-3 pt-3">
        <div className="flex items-center gap-1 overflow-x-auto [scrollbar-width:none]">
          {CATEGORY_TABS.map((tab) => (
            <Button
              className="h-7 shrink-0 rounded-full px-3 text-xs"
              data-active={category === tab.value}
              key={tab.value}
              onClick={() => setCategory(tab.value)}
              size="sm"
              variant={category === tab.value ? "secondary" : "ghost"}
            >
              {tab.label}
              {counts[tab.value] > 0 ? (
                <span className="ml-1 text-muted-foreground">
                  {counts[tab.value]}
                </span>
              ) : null}
            </Button>
          ))}
        </div>

        <div className="flex items-center gap-2">
          <div className="relative flex-1">
            <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
            <input
              className="h-8 w-full rounded-md border border-border bg-background pl-8 pr-3 text-xs placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder="Search files..."
              type="text"
              value={searchQuery}
            />
            {searchQuery ? (
              <button
                aria-label="Clear search"
                className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                onClick={() => setSearchQuery("")}
                type="button"
              >
                ×
              </button>
            ) : null}
          </div>

          <div className="relative">
            <select
              aria-label="Sort files"
              className="h-8 appearance-none rounded-md border border-border bg-background px-7 py-0 pr-6 text-xs focus:outline-none focus:ring-1 focus:ring-ring"
              onChange={(e) => setSort(e.target.value as FileSort)}
              value={sort}
            >
              {SORT_OPTIONS.map((opt) => (
                <option key={opt.value} value={opt.value}>
                  {opt.label}
                </option>
              ))}
            </select>
            <ArrowUpDown className="pointer-events-none absolute left-2 top-1/2 h-3 w-3 -translate-y-1/2 text-muted-foreground" />
          </div>

          {onCreateFolder ? (
            <Button
              className="h-8 shrink-0 gap-1 px-2 text-xs"
              onClick={() => setIsCreatingFolder(true)}
              size="sm"
              variant="outline"
            >
              <FolderPlus className="h-3.5 w-3.5" />
              New
            </Button>
          ) : null}
        </div>

        {isCreatingFolder ? (
          <div className="flex items-center gap-2">
            <input
              autoFocus
              className="h-8 flex-1 rounded-md border border-border bg-background px-3 text-xs focus:outline-none focus:ring-1 focus:ring-ring"
              onChange={(e) => setNewFolderName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void handleCreateFolder();
                if (e.key === "Escape") setIsCreatingFolder(false);
              }}
              placeholder="Folder name..."
              type="text"
              value={newFolderName}
            />
            <Button
              className="h-8 px-3 text-xs"
              disabled={!newFolderName.trim()}
              onClick={() => void handleCreateFolder()}
              size="sm"
            >
              Create
            </Button>
            <Button
              className="h-8 px-2 text-xs"
              onClick={() => setIsCreatingFolder(false)}
              size="sm"
              variant="ghost"
            >
              Cancel
            </Button>
          </div>
        ) : null}
      </div>

      {/* Bulk action bar — below toolbar, above file list */}
      {selectedCount > 0 ? (
        <div className="flex shrink-0 items-center gap-2 border-b border-primary/20 bg-primary/5 px-4 py-2">
          <span className="text-xs font-medium">
            {selectedCount} selected
          </span>
          <Button
            className="h-7 px-2 text-xs"
            onClick={selectAll}
            size="sm"
            variant="ghost"
          >
            Select all
          </Button>
          <div className="flex-1" />
          {selectedInFolder ? (
            <Button
              className="h-7 gap-1 px-2 text-xs"
              onClick={() => void handleBulkRemoveFromFolder()}
              size="sm"
              variant="outline"
            >
              <Undo2 className="h-3.5 w-3.5" />
              Remove from folder
            </Button>
          ) : (
            <div className="relative">
              <select
                aria-label="Move selected files to folder"
                className="h-7 appearance-none rounded border border-border bg-background pl-2 pr-6 text-xs focus:outline-none focus:ring-1 focus:ring-ring"
                onChange={(e) => {
                  if (e.target.value) void handleBulkMoveToFolder(e.target.value);
                }}
                value=""
              >
                <option disabled value="">
                  Move to folder…
                </option>
                {folders.map((f) => (
                  <option key={f.dTag} value={f.dTag}>
                    {f.name}
                  </option>
                ))}
              </select>
              <FolderInput className="pointer-events-none absolute right-2 top-1/2 h-3 w-3 -translate-y-1/2 text-muted-foreground" />
            </div>
          )}
          <Button
            className="h-7 px-2 text-xs"
            onClick={() => setSelectedIds(new Set())}
            size="sm"
            variant="ghost"
          >
            <X className="mr-1 h-3.5 w-3.5" />
            Clear
          </Button>
        </div>
      ) : null}

      {/* File list */}
      <div
        className="flex-1 overflow-y-auto"
        onDragOver={(e) => {
          // Accept folder drops to un-nest (move to root)
          if (e.dataTransfer.types.includes("application/x-folder")) {
            e.preventDefault();
            e.dataTransfer.dropEffect = "move";
          }
        }}
        onDrop={(e) => {
          const folderDTag = e.dataTransfer.getData("application/x-folder");
          if (folderDTag) {
            e.preventDefault();
            const folder = folders.find((f) => f.dTag === folderDTag);
            if (folder?.parentDTag) {
              void onSetFolderParent?.(folder, undefined);
            }
          }
        }}
      >
        {filtered.length === 0 && folders.length === 0 ? (
          <div className="flex items-center justify-center p-12">
            <div className="flex max-w-xs flex-col items-center gap-2 text-center">
              <p className="text-sm font-medium">No files yet</p>
              <p className="text-xs text-muted-foreground">
                Files shared in this channel will appear here.
              </p>
            </div>
          </div>
        ) : (
          <div className="divide-y divide-border py-1">
            {flatFolders.map(({ folder, depth }) => {
              const folderFiles = filesByFolder.get(folder.dTag) ?? [];
              const isExpanded = expandedFolders.has(folder.dTag);

              return (
                <div key={folder.dTag}>
                  <div
                    className={`flex items-center gap-2 px-3 py-2 transition-colors ${
                      dragOverFolder === folder.dTag
                        ? "bg-primary/10 ring-2 ring-primary/30"
                        : "hover:bg-muted/50"
                    }`}
                    draggable
                    onDragLeave={handleFolderDragLeave}
                    onDragOver={(e) => handleFolderDragOver(e, folder.dTag)}
                    onDragStart={(e) => {
                      e.dataTransfer.setData("application/x-folder", folder.dTag);
                      e.dataTransfer.effectAllowed = "move";
                    }}
                    onDrop={(e) => void handleFolderDrop(e, folder)}
                    style={{ paddingLeft: `${12 + depth * 20}px` }}
                  >
                    <button
                      className="flex flex-1 items-center gap-2 text-sm font-medium"
                      onClick={() => toggleFolder(folder.dTag)}
                      type="button"
                    >
                      {isExpanded ? (
                        <ChevronDown className="h-4 w-4 text-muted-foreground" />
                      ) : (
                        <ChevronRight className="h-4 w-4 text-muted-foreground" />
                      )}
                      <Folder className="h-4 w-4 text-muted-foreground" />
                      {folder.name}
                      <span className="text-xs text-muted-foreground">
                        ({folderFiles.length})
                      </span>
                    </button>
                    {onDeleteFolder ? (
                      <Button
                        aria-label={`Delete folder ${folder.name}`}
                        className="h-7 w-7 opacity-50 hover:opacity-100"
                        onClick={() => void onDeleteFolder(folder)}
                        size="icon-xs"
                        variant="ghost"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </Button>
                    ) : null}
                  </div>
                  {isExpanded ? (
                    <div className="divide-y divide-border border-l-2 border-l-muted ml-6">
                      {folderFiles.length === 0 && !folderTree.has(folder.dTag) ? (
                        <p className="px-3 py-4 text-xs text-muted-foreground">
                          Empty folder — drag files here or use checkboxes to add them.
                        </p>
                      ) : (
                        folderFiles.map((file) => (
                          <div className="group flex items-center" key={file.key}>
                            <div className="flex-1">
                              {renderFileRow(file)}
                            </div>
                            {onRemoveFileFromFolder ? (
                              <Button
                                aria-label="Remove from folder"
                                className="mr-2 h-7 w-7 shrink-0 opacity-0 transition-opacity group-hover:opacity-100"
                                onClick={() =>
                                  void onRemoveFileFromFolder(folder, file.eventId)
                                }
                                size="icon-xs"
                                variant="ghost"
                              >
                                <Undo2 className="h-3.5 w-3.5" />
                              </Button>
                            ) : null}
                          </div>
                        ))
                      )}
                    </div>
                  ) : null}
                </div>
              );
            })}

            {dragOverFolder ? (
              <div className="px-3 py-1.5 text-xs text-muted-foreground">
                Drop file to add to folder
              </div>
            ) : null}

            {unfiledFiles.map((file) => renderFileRow(file))}
          </div>
        )}
      </div>
    </div>
  );
}
