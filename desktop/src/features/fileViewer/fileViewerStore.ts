/**
 * Open-tab state for the message-attachment file viewer.
 *
 * Lives outside React because the open action fires from inside the memoized
 * markdown renderer (`FileCard`): threading a callback through props there
 * would make every timeline row re-render (see the `React.memo` note in
 * AGENTS.md).
 *
 * Tabs are keyed by the attachment's Blossom URL, which is content-addressed
 * (`/media/{sha256}.{ext}`), so one URL means one file and reopening an
 * attachment activates its existing tab.
 *
 * Community-scoped: `resetFileViewerStore` is wired into
 * `resetCommunityState()` so tabs never leak across communities.
 */

export type FileViewerTab = {
  filename: string;
  /** imeta `m` MIME, when the message carried one. */
  mime?: string;
  /** imeta `size` in bytes, when the message carried one. */
  size?: number;
  /** Blossom media URL — the tab identity. */
  url: string;
};

export type FileViewerState = {
  /** URL of the tab on screen. Survives a close so reopening restores it. */
  activeUrl: string | null;
  /** Whether the panel is visible. Tabs survive a close. */
  isOpen: boolean;
  tabs: readonly FileViewerTab[];
};

const INITIAL_STATE: FileViewerState = {
  activeUrl: null,
  isOpen: false,
  tabs: [],
};

// Reference-stable snapshot for useSyncExternalStore: a fresh object per read
// would re-render subscribers forever.
let state: FileViewerState = INITIAL_STATE;

const listeners = new Set<() => void>();

/**
 * Mounted panel hosts. `FileCard` checks this before opening: with no host
 * (e.g. a forum post route) it falls back to downloading rather than opening
 * a panel nothing would render.
 */
let hostCount = 0;

function setState(next: FileViewerState): void {
  state = next;
  for (const listener of listeners) listener();
}

export function subscribeFileViewer(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getFileViewerState(): FileViewerState {
  return state;
}

/** The tab the panel should render, or `null` when nothing is on screen. */
export function selectActiveFileViewerTab(
  snapshot: FileViewerState,
): FileViewerTab | null {
  if (!snapshot.isOpen) return null;
  return snapshot.tabs.find((tab) => tab.url === snapshot.activeUrl) ?? null;
}

/** Register a mounted panel host. Returns the matching unregister cleanup. */
export function registerFileViewerHost(): () => void {
  hostCount += 1;
  return () => {
    hostCount -= 1;
  };
}

export function hasFileViewerHost(): boolean {
  return hostCount > 0;
}

/**
 * Open an attachment: adds a tab when the URL is new, activates it, and opens
 * the panel. An existing tab keeps its original metadata — the URL already
 * pins the bytes.
 */
export function openFileViewerTab(tab: FileViewerTab): void {
  const exists = state.tabs.some((t) => t.url === tab.url);
  if (exists && state.isOpen && state.activeUrl === tab.url) return;
  setState({
    activeUrl: tab.url,
    isOpen: true,
    tabs: exists ? state.tabs : [...state.tabs, tab],
  });
}

export function activateFileViewerTab(url: string): void {
  if (url === state.activeUrl) return;
  if (!state.tabs.some((t) => t.url === url)) return;
  setState({ ...state, activeUrl: url });
}

/**
 * Close one tab. Closing the active tab activates its right neighbour, else
 * its left one; closing the last tab closes the panel.
 */
export function closeFileViewerTab(url: string): void {
  const index = state.tabs.findIndex((t) => t.url === url);
  if (index === -1) return;
  const tabs = state.tabs.filter((t) => t.url !== url);
  if (tabs.length === 0) {
    setState({ activeUrl: null, isOpen: false, tabs: [] });
    return;
  }
  const activeUrl =
    state.activeUrl === url
      ? tabs[Math.min(index, tabs.length - 1)].url
      : state.activeUrl;
  setState({ ...state, activeUrl, tabs });
}

/** Hide the panel, keeping tabs so a later open restores them. */
export function closeFileViewer(): void {
  if (!state.isOpen) return;
  setState({ ...state, isOpen: false });
}

/** Community switch: drop every tab. Called from `resetCommunityState`. */
export function resetFileViewerStore(): void {
  setState(INITIAL_STATE);
}
