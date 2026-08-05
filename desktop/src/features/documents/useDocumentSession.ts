/**
 * Tabs, autosave, and watcher reconciliation for the Documents editor.
 *
 * The two rules worth stating up front, because both protect the user's files:
 *
 *  1. **Nothing is written unless the user edited it.** Dirtiness is derived by
 *     comparing the projected bytes against what was read from disk, so opening
 *     a note — or undoing back to its original text — never triggers a save.
 *
 *  2. **A dirty buffer is never silently replaced.** Onyx's watcher rule is
 *     "external edits always win", which is right for a clean tab and wrong for
 *     one with unsaved work. Here a genuine external change to a dirty tab
 *     raises a banner and lets the user choose.
 */
import * as React from "react";
import { toast } from "sonner";

import {
  deleteVaultEntry,
  onVaultFileModified,
  onVaultFilesChanged,
  readVaultFile,
  startVaultWatch,
  stopVaultWatch,
  writeVaultFile,
} from "@/shared/api/vault";
import { useVaultInvalidation } from "@/features/documents/hooks";
import { useAlwaysLivePreview } from "@/features/documents/useDocumentsPreferences";
import {
  activeTab as selectActiveTab,
  closeTab as closeTabIn,
  closeTabsUnder,
  type DocumentTab,
  type DocumentViewMode,
  emptyTabsState,
  findTab,
  markTabSaved,
  openTab as openTabIn,
  reloadTabFromDisk,
  renameTabPath,
  setTabContent,
  setTabViewMode,
  tabLabel,
} from "@/features/documents/lib/documentTabs";
import {
  joinFrontmatter,
  splitFrontmatter,
} from "@/features/documents/lib/frontmatter";
import { reserializeMarkdown } from "@/features/documents/lib/markdownRoundTrip";
import {
  classifyRoundTrip,
  initialViewModeFor,
} from "@/features/documents/lib/roundTripGuard";

const AUTOSAVE_DELAY_MS = 2000;
/** Debounce for tree-shape changes, matching Onyx. */
const TREE_REFRESH_DEBOUNCE_MS = 500;

/** A tab whose file changed on disk while it had unsaved edits. */
export type ExternalChange = { path: string };

function buildTab(
  path: string,
  raw: string,
  alwaysLivePreview: boolean,
): DocumentTab {
  const { body, frontmatter } = splitFrontmatter(raw);
  // The file is still classified either way; the preference only decides which
  // mode we start in, so the warning can still be shown.
  const roundTrip = classifyRoundTrip(body, reserializeMarkdown);
  return {
    content: body,
    diskContent: raw,
    frontmatter,
    isDirty: false,
    name: tabLabel(path),
    path,
    roundTrip,
    viewMode: alwaysLivePreview ? "live" : initialViewModeFor(roundTrip),
  };
}

export function useDocumentSession(vaultRoot: string | null) {
  const alwaysLivePreview = useAlwaysLivePreview();
  // Read through a ref so callbacks do not need to re-create when it changes.
  const alwaysLivePreviewRef = React.useRef(alwaysLivePreview);
  alwaysLivePreviewRef.current = alwaysLivePreview;

  const [state, setState] = React.useState(emptyTabsState);
  const [externalChanges, setExternalChanges] = React.useState<
    ReadonlyMap<string, ExternalChange>
  >(() => new Map());
  const { invalidateTree } = useVaultInvalidation(vaultRoot);

  // Pending autosave timers, keyed by path. Keyed by path and not index so
  // reordering or closing a tab can never cross-save into the wrong file.
  const timersRef = React.useRef(new Map<string, number>());
  // mtimes this app wrote, so the watcher can ignore its own echo.
  const writtenMtimesRef = React.useRef(new Map<string, number>());
  // Mirrors `state` for use inside callbacks that must not re-subscribe.
  const stateRef = React.useRef(state);
  stateRef.current = state;

  const clearTimer = React.useCallback((path: string) => {
    const timer = timersRef.current.get(path);
    if (timer !== undefined) {
      window.clearTimeout(timer);
      timersRef.current.delete(path);
    }
  }, []);

  /** Writes one tab if it is dirty. Safe to call redundantly. */
  const saveTab = React.useCallback(
    async (path: string) => {
      clearTimer(path);
      const tab = findTab(stateRef.current, path);
      if (!tab?.isDirty) return;

      const bytes = joinFrontmatter(tab.frontmatter, tab.content);
      try {
        const { modifiedMs } = await writeVaultFile(path, bytes);
        writtenMtimesRef.current.set(path, modifiedMs);
        setState((current) => markTabSaved(current, path, bytes));
      } catch (error: unknown) {
        toast.error(
          error instanceof Error ? error.message : "Could not save that note.",
        );
      }
    },
    [clearTimer],
  );

  const saveAllDirty = React.useCallback(async () => {
    await Promise.all(
      stateRef.current.tabs
        .filter((tab) => tab.isDirty)
        .map((tab) => saveTab(tab.path)),
    );
  }, [saveTab]);

  /** Records an edit and (re)arms this file's autosave timer. */
  const updateTabContent = React.useCallback(
    (path: string, content: string) => {
      const tab = findTab(stateRef.current, path);
      if (!tab) return;

      const projection = joinFrontmatter(tab.frontmatter, content);
      setState((current) => setTabContent(current, path, content, projection));

      clearTimer(path);
      const timer = window.setTimeout(() => {
        void saveTab(path);
      }, AUTOSAVE_DELAY_MS);
      timersRef.current.set(path, timer);
    },
    [clearTimer, saveTab],
  );

  /** Re-opens a set of paths without stealing focus from `activePath`. */
  const restoreFiles = React.useCallback(
    async (paths: readonly string[], activePath: string | null) => {
      for (const path of paths) {
        try {
          const raw = await readVaultFile(path);
          setState((current) =>
            openTabIn(
              current,
              buildTab(path, raw, alwaysLivePreviewRef.current),
            ),
          );
        } catch {
          // A note deleted since last session simply does not come back.
        }
      }
      if (activePath) {
        setState((current) =>
          findTab(current, activePath) ? { ...current, activePath } : current,
        );
      }
    },
    [],
  );

  const openFile = React.useCallback(async (path: string) => {
    if (findTab(stateRef.current, path)) {
      setState((current) =>
        openTabIn(current, findTab(current, path) as DocumentTab),
      );
      return;
    }
    try {
      const raw = await readVaultFile(path);
      setState((current) =>
        openTabIn(current, buildTab(path, raw, alwaysLivePreviewRef.current)),
      );
    } catch (error: unknown) {
      toast.error(
        error instanceof Error ? error.message : "Could not open that note.",
      );
    }
  }, []);

  /** Closes a tab, flushing any pending save first so edits are not lost. */
  const closeFile = React.useCallback(
    async (path: string) => {
      await saveTab(path);
      clearTimer(path);
      setState((current) => closeTabIn(current, path));
      setExternalChanges((current) => {
        if (!current.has(path)) return current;
        const next = new Map(current);
        next.delete(path);
        return next;
      });
    },
    [clearTimer, saveTab],
  );

  const activateFile = React.useCallback((path: string) => {
    setState((current) =>
      openTabIn(current, findTab(current, path) as DocumentTab),
    );
  }, []);

  const setViewMode = React.useCallback(
    (path: string, viewMode: DocumentViewMode) => {
      setState((current) => setTabViewMode(current, path, viewMode));
    },
    [],
  );

  /** Re-reads a file from disk, discarding the local buffer. */
  const reloadFile = React.useCallback(async (path: string) => {
    try {
      const raw = await readVaultFile(path);
      const rebuilt = buildTab(path, raw, alwaysLivePreviewRef.current);
      setState((current) =>
        reloadTabFromDisk(current, path, {
          content: rebuilt.content,
          diskContent: rebuilt.diskContent,
          frontmatter: rebuilt.frontmatter,
          roundTrip: rebuilt.roundTrip,
        }),
      );
      setExternalChanges((current) => {
        const next = new Map(current);
        next.delete(path);
        return next;
      });
    } catch (error: unknown) {
      toast.error(
        error instanceof Error ? error.message : "Could not reload that note.",
      );
    }
  }, []);

  /** Dismisses the external-change banner, keeping the local buffer. */
  const keepLocalVersion = React.useCallback((path: string) => {
    setExternalChanges((current) => {
      const next = new Map(current);
      next.delete(path);
      return next;
    });
  }, []);

  const deleteEntry = React.useCallback(
    async (path: string) => {
      try {
        await deleteVaultEntry(path);
        // Drop buffers for files that no longer exist, including a whole
        // folder's worth when a directory was removed.
        setState((current) => closeTabsUnder(current, path));
        invalidateTree();
      } catch (error: unknown) {
        toast.error(
          error instanceof Error
            ? error.message
            : "Could not delete that item.",
        );
      }
    },
    [invalidateTree],
  );

  /** Keeps an open buffer pointed at its file after a rename or move. */
  const notePathRenamed = React.useCallback(
    (fromPath: string, toPath: string) => {
      setState((current) => renameTabPath(current, fromPath, toPath));
    },
    [],
  );

  // --- Filesystem watcher -------------------------------------------------

  React.useEffect(() => {
    if (!vaultRoot) return;

    let disposed = false;
    const disposers: Array<() => void> = [];
    let treeTimer: number | undefined;

    void (async () => {
      try {
        await startVaultWatch();
      } catch {
        // Watching is a convenience; the editor still works without it.
        return;
      }
      if (disposed) {
        void stopVaultWatch();
        return;
      }

      disposers.push(
        await onVaultFileModified((entries) => {
          for (const entry of entries) {
            const tab = findTab(stateRef.current, entry.path);
            if (!tab) continue;

            // Our own write echoing back through the watcher.
            const written = writtenMtimesRef.current.get(entry.path);
            if (written !== undefined && written === entry.modifiedMs) continue;

            if (tab.isDirty) {
              // Never clobber unsaved work — cancel the pending save and ask.
              clearTimer(entry.path);
              setExternalChanges((current) => {
                const next = new Map(current);
                next.set(entry.path, { path: entry.path });
                return next;
              });
            } else {
              void reloadFile(entry.path);
            }
          }
        }),
      );

      disposers.push(
        await onVaultFilesChanged(() => {
          window.clearTimeout(treeTimer);
          treeTimer = window.setTimeout(
            invalidateTree,
            TREE_REFRESH_DEBOUNCE_MS,
          );
        }),
      );
    })();

    return () => {
      disposed = true;
      window.clearTimeout(treeTimer);
      for (const dispose of disposers) dispose();
      void stopVaultWatch();
    };
  }, [clearTimer, invalidateTree, reloadFile, vaultRoot]);

  // Read through a ref so the unmount effect below can stay `[]`-scoped and
  // still call the current implementation.
  const saveAllDirtyRef = React.useRef(saveAllDirty);
  saveAllDirtyRef.current = saveAllDirty;

  // On unmount, cancel the pending timers and then flush what they were going
  // to write. Leaving the timers armed would fire against an unmounted
  // component; cancelling them *without* flushing would silently drop the last
  // edit whenever the user leaves Documents inside the 2s debounce window,
  // which is precisely the data loss this module exists to prevent.
  //
  // The writes are deliberately not awaited — a cleanup function cannot be
  // async. They are already in flight at the Tauri boundary by the time React
  // finishes tearing down, and `saveTab`'s `setState` is a harmless no-op once
  // unmounted.
  React.useEffect(() => {
    const timers = timersRef.current;
    return () => {
      for (const timer of timers.values()) {
        window.clearTimeout(timer);
      }
      timers.clear();
      void saveAllDirtyRef.current();
    };
  }, []);

  return {
    activateFile,
    activeTab: selectActiveTab(state),
    closeFile,
    deleteEntry,
    externalChanges,
    keepLocalVersion,
    notePathRenamed,
    openFile,
    reloadFile,
    restoreFiles,
    saveAllDirty,
    saveTab,
    setViewMode,
    state,
    updateTabContent,
  };
}
