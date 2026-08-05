/**
 * Create / rename / delete for vault entries.
 *
 * Each mutation invalidates the tree so the sidebar reflects disk. The
 * filesystem watcher would eventually do the same, but waiting up to a second
 * for a poll makes the UI feel broken.
 */
import * as React from "react";
import { toast } from "sonner";

import {
  createVaultFile,
  createVaultFolder,
  renameVaultEntry,
} from "@/shared/api/vault";
import { useVaultInvalidation } from "@/features/documents/hooks";
import {
  baseName,
  joinPath,
  parentOf,
} from "@/features/documents/lib/treeModel";

/** Appends `.md` unless the user already typed a markdown extension. */
export function withMarkdownExtension(name: string): string {
  return /\.(?:md|markdown)$/i.test(name) ? name : `${name}.md`;
}

/**
 * Validates a new file or folder name.
 *
 * Path separators are rejected rather than silently creating nested folders,
 * and leading dots are rejected because the tree hides dotted entries — a note
 * named `.private` would vanish the moment it was created.
 */
export function nameError(name: string): string | null {
  const trimmed = name.trim();
  if (!trimmed) return "Enter a name.";
  if (/[/\\]/.test(trimmed)) return "Names cannot contain slashes.";
  if (trimmed.startsWith(".")) return "Names cannot start with a dot.";
  if (trimmed === "." || trimmed === "..") return "That name is not allowed.";
  return null;
}

export function useVaultMutations(vaultRoot: string | null) {
  const { invalidateTree } = useVaultInvalidation(vaultRoot);

  const createNote = React.useCallback(
    async (directory: string, name: string): Promise<string | null> => {
      const path = joinPath(directory, withMarkdownExtension(name.trim()));
      try {
        await createVaultFile(path);
        invalidateTree();
        return path;
      } catch (error: unknown) {
        toast.error(
          error instanceof Error
            ? error.message
            : "Could not create that note.",
        );
        return null;
      }
    },
    [invalidateTree],
  );

  const createFolder = React.useCallback(
    async (directory: string, name: string): Promise<string | null> => {
      const path = joinPath(directory, name.trim());
      try {
        await createVaultFolder(path);
        invalidateTree();
        return path;
      } catch (error: unknown) {
        toast.error(
          error instanceof Error
            ? error.message
            : "Could not create that folder.",
        );
        return null;
      }
    },
    [invalidateTree],
  );

  /** Renames in place, keeping the entry in its current folder. */
  const rename = React.useCallback(
    async (
      path: string,
      nextName: string,
      isDirectory: boolean,
    ): Promise<string | null> => {
      const finalName = isDirectory
        ? nextName.trim()
        : withMarkdownExtension(nextName.trim());
      if (finalName === baseName(path)) return path;

      const nextPath = joinPath(parentOf(path), finalName);
      try {
        await renameVaultEntry(path, nextPath);
        invalidateTree();
        return nextPath;
      } catch (error: unknown) {
        toast.error(
          error instanceof Error
            ? error.message
            : "Could not rename that item.",
        );
        return null;
      }
    },
    [invalidateTree],
  );

  return { createFolder, createNote, rename };
}
