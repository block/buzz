import type * as React from "react";

import type { VaultEntry } from "@/shared/api/vaultTypes";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/shared/ui/context-menu";

export type TreeMenuTarget = {
  entry: VaultEntry;
};

/**
 * Right-click menu for a tree row.
 *
 * Uses the shared Radix primitives rather than Onyx's hand-rolled menu (which
 * positions itself from raw x/y coordinates and flips manually near the
 * viewport edge) — Radix already handles collision, focus, and dismissal.
 */
export function DocumentTreeContextMenu({
  children,
  entry,
  onCreateFolderIn,
  onCreateNoteIn,
  onDelete,
  onRename,
}: {
  children: React.ReactNode;
  entry: VaultEntry;
  onCreateFolderIn: (entry: VaultEntry) => void;
  onCreateNoteIn: (entry: VaultEntry) => void;
  onDelete: (entry: VaultEntry) => void;
  onRename: (entry: VaultEntry) => void;
}) {
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>{children}</ContextMenuTrigger>
      <ContextMenuContent className="w-48">
        <ContextMenuItem
          data-testid="documents-menu-new-note"
          onSelect={() => onCreateNoteIn(entry)}
        >
          New note
        </ContextMenuItem>
        <ContextMenuItem
          data-testid="documents-menu-new-folder"
          onSelect={() => onCreateFolderIn(entry)}
        >
          New folder
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem
          data-testid="documents-menu-rename"
          onSelect={() => onRename(entry)}
        >
          Rename
        </ContextMenuItem>
        <ContextMenuItem
          className="text-destructive focus:text-destructive"
          data-testid="documents-menu-delete"
          onSelect={() => onDelete(entry)}
        >
          Delete
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

/**
 * Delete confirmation.
 *
 * Deleting a folder takes everything under it, so the copy says so explicitly
 * rather than leaving the user to infer it.
 */
export function DocumentDeleteDialog({
  entry,
  onCancel,
  onConfirm,
}: {
  entry: VaultEntry | null;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <AlertDialog
      onOpenChange={(open) => !open && onCancel()}
      open={Boolean(entry)}
    >
      <AlertDialogContent data-testid="documents-delete-dialog">
        <AlertDialogHeader>
          <AlertDialogTitle>Delete “{entry?.name}”?</AlertDialogTitle>
          <AlertDialogDescription>
            {entry?.isDirectory
              ? "This deletes the folder and every note inside it. This cannot be undone."
              : "This deletes the note from your vault. This cannot be undone."}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel onClick={onCancel}>Cancel</AlertDialogCancel>
          <AlertDialogAction
            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            data-testid="documents-delete-confirm"
            onClick={onConfirm}
          >
            Delete
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
