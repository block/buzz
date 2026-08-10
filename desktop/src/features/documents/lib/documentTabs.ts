/**
 * Pure tab-state model for the Documents editor.
 *
 * Kept free of React and Tauri so the index arithmetic — the part that
 * historically goes wrong — can be tested directly.
 */
import {
  baseName,
  stripMarkdownExtension,
} from "@/features/documents/lib/treeModel";
import type { RoundTripStatus } from "@/features/documents/lib/roundTripGuard";

export type DocumentViewMode = "live" | "source";

export type DocumentTab = {
  /** Absolute path. The identity key everywhere — never the index. */
  path: string;
  /** Basename without the markdown extension, for the tab label. */
  name: string;
  /** Live editor content: the body only, with frontmatter split off. */
  content: string;
  /** The frontmatter block, re-attached verbatim on save. */
  frontmatter: string | null;
  /** Exact bytes last read from or written to disk — the dirty comparand. */
  diskContent: string;
  isDirty: boolean;
  roundTrip: RoundTripStatus;
  viewMode: DocumentViewMode;
};

export type DocumentTabsState = {
  tabs: DocumentTab[];
  /** Path of the active tab, or `null` when none are open. */
  activePath: string | null;
};

export const emptyTabsState: DocumentTabsState = {
  activePath: null,
  tabs: [],
};

export function tabLabel(path: string): string {
  return stripMarkdownExtension(baseName(path));
}

export function findTab(
  state: DocumentTabsState,
  path: string,
): DocumentTab | null {
  return state.tabs.find((tab) => tab.path === path) ?? null;
}

export function activeTab(state: DocumentTabsState): DocumentTab | null {
  return state.activePath ? findTab(state, state.activePath) : null;
}

export function hasDirtyTabs(state: DocumentTabsState): boolean {
  return state.tabs.some((tab) => tab.isDirty);
}

export function dirtyPaths(state: DocumentTabsState): string[] {
  return state.tabs.filter((tab) => tab.isDirty).map((tab) => tab.path);
}

/**
 * Opens `tab`, or focuses it when already open.
 *
 * Re-opening never clobbers an existing tab's buffer: a dirty tab whose file is
 * clicked again in the tree must keep the user's unsaved edits.
 */
export function openTab(
  state: DocumentTabsState,
  tab: DocumentTab,
): DocumentTabsState {
  const existing = findTab(state, tab.path);
  if (existing) {
    return { ...state, activePath: existing.path };
  }
  return { activePath: tab.path, tabs: [...state.tabs, tab] };
}

/**
 * Closes a tab and picks the next active one.
 *
 * When the closed tab was active, focus moves to its right-hand neighbour,
 * falling back to the left when it was last. Computed from the *new* array
 * rather than a stale length — the index bug this model exists to avoid.
 */
export function closeTab(
  state: DocumentTabsState,
  path: string,
): DocumentTabsState {
  const index = state.tabs.findIndex((tab) => tab.path === path);
  if (index === -1) return state;

  const tabs = state.tabs.filter((tab) => tab.path !== path);
  if (tabs.length === 0) {
    return emptyTabsState;
  }

  if (state.activePath !== path) {
    return { ...state, tabs };
  }

  const nextIndex = Math.min(index, tabs.length - 1);
  return { activePath: tabs[nextIndex].path, tabs };
}

export function activateTab(
  state: DocumentTabsState,
  path: string,
): DocumentTabsState {
  return findTab(state, path) ? { ...state, activePath: path } : state;
}

/** Applies `update` to one tab, leaving the rest untouched. */
export function updateTab(
  state: DocumentTabsState,
  path: string,
  update: (tab: DocumentTab) => DocumentTab,
): DocumentTabsState {
  let changed = false;
  const tabs = state.tabs.map((tab) => {
    if (tab.path !== path) return tab;
    const next = update(tab);
    if (next !== tab) changed = true;
    return next;
  });
  return changed ? { ...state, tabs } : state;
}

/**
 * Records an edit from the editor.
 *
 * Dirtiness is derived by comparing against `diskContent` rather than latched,
 * so undoing back to the on-disk text correctly clears the flag instead of
 * leaving a permanently dirty tab.
 */
export function setTabContent(
  state: DocumentTabsState,
  path: string,
  content: string,
  /** The exact bytes this content would produce on disk. */
  diskProjection: string,
): DocumentTabsState {
  return updateTab(state, path, (tab) => {
    if (tab.content === content) return tab;
    return {
      ...tab,
      content,
      isDirty: diskProjection !== tab.diskContent,
    };
  });
}

/** Marks a tab saved, adopting the bytes that were written. */
export function markTabSaved(
  state: DocumentTabsState,
  path: string,
  diskContent: string,
): DocumentTabsState {
  return updateTab(state, path, (tab) => ({
    ...tab,
    diskContent,
    isDirty: false,
  }));
}

/** Replaces a tab's buffer from disk, discarding local edits. */
export function reloadTabFromDisk(
  state: DocumentTabsState,
  path: string,
  next: Pick<
    DocumentTab,
    "content" | "diskContent" | "frontmatter" | "roundTrip"
  >,
): DocumentTabsState {
  return updateTab(state, path, (tab) => ({
    ...tab,
    ...next,
    isDirty: false,
  }));
}

export function setTabViewMode(
  state: DocumentTabsState,
  path: string,
  viewMode: DocumentViewMode,
): DocumentTabsState {
  return updateTab(state, path, (tab) =>
    tab.viewMode === viewMode ? tab : { ...tab, viewMode },
  );
}

/**
 * Rewrites a tab's identity after its file is renamed or moved on disk, so the
 * open buffer follows the file instead of pointing at a path that no longer
 * exists.
 */
export function renameTabPath(
  state: DocumentTabsState,
  fromPath: string,
  toPath: string,
): DocumentTabsState {
  if (!findTab(state, fromPath)) return state;

  const tabs = state.tabs.map((tab) =>
    tab.path === fromPath
      ? { ...tab, name: tabLabel(toPath), path: toPath }
      : tab,
  );
  return {
    activePath: state.activePath === fromPath ? toPath : state.activePath,
    tabs,
  };
}

/**
 * Closes every tab under `prefix` — used when a folder is deleted, so buffers
 * for files that no longer exist do not linger.
 */
export function closeTabsUnder(
  state: DocumentTabsState,
  prefix: string,
): DocumentTabsState {
  const doomed = state.tabs.filter(
    (tab) => tab.path === prefix || tab.path.startsWith(`${prefix}/`),
  );
  return doomed.reduce((current, tab) => closeTab(current, tab.path), state);
}
