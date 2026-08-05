import { ChevronDown, ChevronRight, FileText, Folder } from "lucide-react";

import type { VaultTreeRow } from "@/features/documents/lib/treeModel";
import { stripMarkdownExtension } from "@/features/documents/lib/treeModel";
import { DocumentTreeContextMenu } from "@/features/documents/ui/DocumentTreeContextMenu";
import type { VaultEntry } from "@/shared/api/vaultTypes";
import { cn } from "@/shared/lib/cn";

const INDENT_PX = 12;

/**
 * The vault file tree.
 *
 * Renders only visible rows (`flattenVisibleRows` drops collapsed subtrees), so
 * a large vault costs what is on screen rather than what is on disk.
 */
export function DocumentTreePane({
  activePath,
  onCreateFolderIn,
  onCreateNoteIn,
  onDelete,
  onRename,
  onSelectFile,
  onToggleFolder,
  rows,
}: {
  activePath: string | null;
  onCreateFolderIn: (entry: VaultEntry) => void;
  onCreateNoteIn: (entry: VaultEntry) => void;
  onDelete: (entry: VaultEntry) => void;
  onRename: (entry: VaultEntry) => void;
  onSelectFile: (path: string) => void;
  onToggleFolder: (path: string) => void;
  rows: VaultTreeRow[];
}) {
  if (rows.length === 0) {
    return (
      <p className="px-3 py-4 text-sm text-muted-foreground">
        No markdown files in this folder yet.
      </p>
    );
  }

  return (
    <ul className="py-1" data-testid="documents-tree">
      {rows.map(({ depth, entry, isExpanded }) => {
        const isActive = !entry.isDirectory && entry.path === activePath;
        const Chevron = isExpanded ? ChevronDown : ChevronRight;

        return (
          <li key={entry.path}>
            <DocumentTreeContextMenu
              entry={entry}
              onCreateFolderIn={onCreateFolderIn}
              onCreateNoteIn={onCreateNoteIn}
              onDelete={onDelete}
              onRename={onRename}
            >
              <button
                className={cn(
                  "flex w-full items-center gap-1.5 rounded-md py-1 pr-2 text-left text-sm",
                  "hover:bg-sidebar-accent/60",
                  isActive
                    ? "bg-sidebar-accent text-sidebar-accent-foreground"
                    : "text-foreground/90",
                )}
                data-testid={
                  entry.isDirectory
                    ? `documents-folder-${entry.name}`
                    : `documents-file-${entry.name}`
                }
                onClick={() =>
                  entry.isDirectory
                    ? onToggleFolder(entry.path)
                    : onSelectFile(entry.path)
                }
                style={{ paddingLeft: `${8 + depth * INDENT_PX}px` }}
                title={entry.name}
                type="button"
              >
                {entry.isDirectory ? (
                  <>
                    <Chevron className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                    <Folder className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  </>
                ) : (
                  <>
                    <span className="h-3.5 w-3.5 shrink-0" />
                    <FileText className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  </>
                )}
                <span className="truncate">
                  {entry.isDirectory
                    ? entry.name
                    : stripMarkdownExtension(entry.name)}
                </span>
              </button>
            </DocumentTreeContextMenu>
          </li>
        );
      })}
    </ul>
  );
}
