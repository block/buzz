import * as React from "react";
import { useQuery } from "@tanstack/react-query";

import {
  type ChannelFileEntry,
  type FileVersionStatus,
  fileVersionStatus,
  listChannelFiles,
} from "@/shared/api/channelFiles";
import {
  buildFileVersionChains,
  buildLatestVersionIndex,
} from "@/features/messages/lib/fileVersionChains.mjs";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";

export type { FileVersionStatus };

/**
 * Everything a file card needs to render and navigate its version chain.
 *
 * `olderVersions` is populated only for the head of a chain — an outdated file
 * gets `latestEventId` to jump forward instead. See `FileVersionBadge` and the
 * `FileCard` history disclosure for why the two are mutually exclusive.
 */
export type FileVersionInfo = {
  status: FileVersionStatus;
  /** The message id of the newest version, when this file is not it. */
  latestEventId: string | null;
  /** Prior versions behind this file, newest-first. Only set on the head. */
  olderVersions: ChannelFileEntry[];
  /** 1-based position in the chain, and its length. */
  position: number;
  total: number;
};

type FileVersionApi = {
  isAvailable: boolean;
  /**
   * Register interest in the version graph. Returns an unsubscribe. The
   * underlying query only runs while at least one consumer is registered —
   * see `FileVersionProvider` for why that matters.
   */
  register: () => () => void;
  infoForUrl: (url: string | null | undefined) => FileVersionInfo | null;
  /**
   * Scroll the timeline to a message and highlight it, when the surface
   * beneath this provider can do that. Null in surfaces that cannot (the
   * Files tab renders its own list rather than a timeline).
   */
  jumpToMessage: ((eventId: string) => void) | null;
};

const NOOP_FILE_VERSIONS: FileVersionApi = {
  isAvailable: false,
  register: () => () => {},
  infoForUrl: () => null,
  jumpToMessage: null,
};

const FileVersionContext =
  React.createContext<FileVersionApi>(NOOP_FILE_VERSIONS);

/**
 * Version info for the file at `url`, or `null` when it isn't known — no
 * provider above this component, the graph hasn't loaded yet, or this file
 * simply has no version links.
 *
 * Mounting this hook is what causes the channel's file graph to be fetched, so
 * call it only from components that actually render a version affordance.
 */
export function useFileVersionInfo(
  url: string | null | undefined,
): FileVersionInfo | null {
  const { isAvailable, register, infoForUrl } =
    React.useContext(FileVersionContext);

  React.useEffect(() => {
    if (!isAvailable) return;
    return register();
  }, [isAvailable, register]);

  return infoForUrl(url);
}

/** Jump the timeline to another version's message, when the surface allows. */
export function useFileVersionJump(): ((eventId: string) => void) | null {
  return React.useContext(FileVersionContext).jumpToMessage;
}

/**
 * Index a channel's files by URL so a card holding only an href can find its
 * own version status.
 *
 * Keyed under both the verbatim imeta `url` and its `rewriteRelayUrl` form,
 * because the two callers arrive with different shapes: `FilesPanel` holds the
 * raw `ChannelFileEntry.url`, while the markdown renderer hands `FileCard` an
 * already-rewritten href (`resolveFileCard`). Today those happen to be equal
 * for every file that reaches a FileCard — `RELAY_MEDIA_RE` only matches
 * image/video extensions, so generic attachments pass through `rewriteRelayUrl`
 * unchanged — but that is a coincidence of the regex, not a contract, and it
 * would break silently (badge quietly stops rendering) the day a previewable
 * extension is added to it. Indexing both costs one extra map entry per file.
 */
function indexByUrl(files: ChannelFileEntry[]): Map<string, FileVersionInfo> {
  const chains = buildFileVersionChains(files);
  const latestByEventId = buildLatestVersionIndex(files);

  // Position within a chain, so an outdated file can say "Version 2 of 3"
  // rather than a bare "Outdated".
  const positions = new Map<string, { position: number; total: number }>();
  const olderByHead = new Map<string, ChannelFileEntry[]>();
  for (const chain of chains) {
    const total = chain.older.length + 1;
    positions.set(chain.latest.eventId, { position: total, total });
    olderByHead.set(chain.latest.eventId, chain.older);
    // `older` is newest-first, so the first entry is one step back from head.
    chain.older.forEach((file, index) => {
      positions.set(file.eventId, { position: total - 1 - index, total });
    });
  }

  const byUrl = new Map<string, FileVersionInfo>();
  for (const file of files) {
    if (!file.url) continue;
    const latestEventId = latestByEventId.get(file.eventId) ?? file.eventId;
    const placement = positions.get(file.eventId) ?? { position: 1, total: 1 };
    const info: FileVersionInfo = {
      status: fileVersionStatus(file),
      latestEventId: latestEventId === file.eventId ? null : latestEventId,
      olderVersions: olderByHead.get(file.eventId) ?? [],
      position: placement.position,
      total: placement.total,
    };
    byUrl.set(file.url, info);
    const rewritten = rewriteRelayUrl(file.url);
    if (rewritten !== file.url) byUrl.set(rewritten, info);
  }
  return byUrl;
}

/**
 * Supplies the channel-wide file version graph to attachment cards rendered
 * beneath it.
 *
 * Why a context rather than props: a message knows nothing about being
 * superseded — that fact lives in a *later* message — so the badge can only
 * come from a channel-wide query, and threading it from the channel down
 * through the markdown renderer to `FileCard` would mean a prop on every
 * intermediate component that has no use for it. `MessageSelectionProvider`
 * solves the same shape the same way, and for the same reason.
 *
 * Why the query is demand-driven: `listChannelFiles` pages backward through
 * the channel's entire history (several sequential relay round-trips), so
 * firing it on every channel open would put a real cost on channels the user
 * never scrolls to a file in. Instead `useFileVersionInfo` registers on
 * mount, and the query is enabled only while at least one card is asking —
 * i.e. when a file is actually on screen. The ref-count only flips React state
 * on the 0↔1 transitions so a virtualized timeline scrolling cards in and out
 * doesn't re-render the provider on every row.
 *
 * The query key matches `FilesPanel`'s exactly, so opening the Files tab and
 * scrolling the timeline share one cache entry rather than fetching twice.
 */
export function FileVersionProvider({
  channelId,
  children,
  jumpToMessage = null,
}: {
  channelId?: string | null;
  children: React.ReactNode;
  /** Supplied by the timeline, which owns scroll; omitted elsewhere. */
  jumpToMessage?: ((eventId: string) => void) | null;
}) {
  const [hasConsumers, setHasConsumers] = React.useState(false);
  const consumerCountRef = React.useRef(0);

  const register = React.useCallback(() => {
    consumerCountRef.current += 1;
    if (consumerCountRef.current === 1) setHasConsumers(true);
    return () => {
      consumerCountRef.current -= 1;
      if (consumerCountRef.current === 0) setHasConsumers(false);
    };
  }, []);

  const filesQuery = useQuery({
    queryKey: ["channel-files", channelId],
    queryFn: () => listChannelFiles(channelId as string),
    enabled: hasConsumers && channelId != null,
    // The graph only changes when someone uploads or links a version, neither
    // of which is frequent enough to justify re-paging the channel's history
    // on every remount as the user moves around.
    staleTime: 60_000,
  });

  const byUrl = React.useMemo(
    () => indexByUrl(filesQuery.data ?? []),
    [filesQuery.data],
  );

  const api = React.useMemo<FileVersionApi>(
    () => ({
      isAvailable: true,
      register,
      infoForUrl: (url) => (url ? (byUrl.get(url) ?? null) : null),
      jumpToMessage,
    }),
    [byUrl, jumpToMessage, register],
  );

  return <FileVersionContext value={api}>{children}</FileVersionContext>;
}
