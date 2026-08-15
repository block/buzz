import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import {
  AlertCircle,
  ChevronRight,
  Download,
  File as FileIcon,
} from "lucide-react";

import {
  type ChannelFileEntry,
  fileVersionStatus,
  isOutdatedFile,
  listChannelFiles,
} from "@/shared/api/channelFiles";
import { buildFileVersionChains } from "@/features/messages/lib/fileVersionChains.mjs";
import { FileVersionBadge } from "@/shared/ui/FileVersionBadge";
import { classifyFilePreviewKind } from "@/shared/ui/markdownFileCard";
import { FilePreviewModal } from "@/shared/ui/filePreview/FilePreviewModal";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { formatItemTimestamp } from "@/shared/lib/datetime";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { truncatePubkey } from "@/shared/lib/pubkey";

/** Human-readable byte size: "820 B", "12.4 KB", "3.1 MB". Matches the copy
 * used in FileCard.tsx/FilePreviewModal.tsx — kept local rather than shared
 * since all three copies are a few lines and none of them import from a
 * common util today. */
function formatFileSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let size = bytes / 1024;
  let i = 0;
  while (size >= 1024 && i < units.length - 1) {
    size /= 1024;
    i += 1;
  }
  return `${size < 10 ? size.toFixed(1) : Math.round(size)} ${units[i]}`;
}

function FileRow({
  file,
  indented = false,
  onOpen,
  versionLabel,
}: {
  file: ChannelFileEntry;
  /** Older versions render inset beneath the head of their chain. */
  indented?: boolean;
  onOpen: (file: ChannelFileEntry) => void;
  /** e.g. "Version 2 of 3" — position within the chain, when it has one. */
  versionLabel?: string;
}) {
  const outdated = isOutdatedFile(file);
  const filename = file.filename ?? "Untitled file";

  return (
    <div
      className={cn(
        "group flex items-start gap-3 rounded-lg border border-border/60 px-3 py-2.5 transition-colors hover:bg-muted/50",
        outdated && "opacity-70",
        indented && "ml-6 border-dashed",
      )}
    >
      <button
        className="flex min-w-0 flex-1 items-start gap-3 text-left"
        onClick={() => onOpen(file)}
        type="button"
      >
        <FileIcon className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-medium text-foreground">
              {filename}
            </span>
            <FileVersionBadge status={fileVersionStatus(file)} />
          </div>
          <div className="mt-0.5 flex items-center gap-2 text-xs text-muted-foreground">
            {versionLabel ? (
              <>
                <span>{versionLabel}</span>
                <span>&middot;</span>
              </>
            ) : null}
            <span>{truncatePubkey(file.uploadedBy)}</span>
            <span>&middot;</span>
            {/* `withTime` — several versions of the same file are routinely
                uploaded on one day, and a date alone can't order them. */}
            <span title={new Date(file.uploadedAt * 1000).toLocaleString()}>
              {formatItemTimestamp(file.uploadedAt, { withTime: true })}
            </span>
            {file.size != null ? (
              <>
                <span>&middot;</span>
                <span>{formatFileSize(file.size)}</span>
              </>
            ) : null}
          </div>
        </div>
      </button>
      <Download className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground/60" />
    </div>
  );
}

/**
 * One version chain: the current file, with any older versions collapsed
 * beneath it behind a disclosure.
 *
 * Only the head carries the disclosure. An older version inside it does not
 * get its own — the chain is a property of the document, and rendering it at
 * every level would show the same history nested inside itself. Each older row
 * still says where it sits ("Version 2 of 3") so the position is not lost.
 */
function FileChainRow({
  chain,
  onOpen,
}: {
  chain: { latest: ChannelFileEntry; older: ChannelFileEntry[] };
  onOpen: (file: ChannelFileEntry) => void;
}) {
  const [expanded, setExpanded] = React.useState(false);
  const total = chain.older.length + 1;

  if (chain.older.length === 0) {
    return <FileRow file={chain.latest} onOpen={onOpen} />;
  }

  return (
    <div className="flex flex-col gap-1">
      <FileRow
        file={chain.latest}
        onOpen={onOpen}
        versionLabel={`Version ${total} of ${total}`}
      />
      <button
        aria-expanded={expanded}
        className="ml-6 flex items-center gap-1 self-start rounded px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
        data-testid="file-chain-toggle"
        onClick={() => setExpanded((value) => !value)}
        type="button"
      >
        <ChevronRight
          className={cn(
            "h-3 w-3 transition-transform",
            expanded && "rotate-90",
          )}
        />
        {expanded
          ? "Hide earlier versions"
          : `${chain.older.length} earlier version${
              chain.older.length === 1 ? "" : "s"
            }`}
      </button>
      {expanded
        ? chain.older.map((older, index) => (
            <FileRow
              file={older}
              indented
              key={older.eventId}
              onOpen={onOpen}
              // `older` is newest-first, so the row just below the head is
              // one version back from the top.
              versionLabel={`Version ${total - 1 - index} of ${total}`}
            />
          ))
        : null}
    </div>
  );
}

/**
 * Per-channel Files tab — lists every file ever shared in the channel, with
 * an "Outdated" badge on any file a later upload was tagged as superseding
 * (see `supersedes`/`supersededBy` on `ChannelFileEntry`).
 *
 * Modeled on `MembersSidebar`: a Radix `Dialog` opened/closed via a boolean
 * prop pair, same as the existing "supplementary channel panel" pattern in
 * `ChannelScreen.tsx` rather than a persistent tab bar — this keeps the
 * change additive (one new sibling overlay + one new toggle button) instead
 * of restructuring `ChannelScreen.tsx`'s message/thread/forum layout.
 */
export function FilesPanel({
  channel,
  open,
  onOpenChange,
}: {
  channel: Channel | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const channelId = channel?.id ?? null;
  const filesQuery = useQuery({
    queryKey: ["channel-files", channelId],
    queryFn: () => listChannelFiles(channelId as string),
    enabled: open && channelId != null,
  });
  const [previewFile, setPreviewFile] = React.useState<ChannelFileEntry | null>(
    null,
  );

  // One row per version chain rather than per file: older versions collapse
  // under the current one. Unversioned files come back as single-entry chains,
  // so there is still only one list to render.
  const chains = React.useMemo(
    () => buildFileVersionChains(filesQuery.data ?? []),
    [filesQuery.data],
  );

  const handleOpenFile = React.useCallback((file: ChannelFileEntry) => {
    if (!file.url) return; // no `url` imeta field — nothing to preview/download
    setPreviewFile(file);
  }, []);

  const previewHref = previewFile?.url
    ? rewriteRelayUrl(previewFile.url)
    : null;
  const previewKind = previewFile?.filename
    ? classifyFilePreviewKind(previewFile.filename)
    : null;

  return (
    <>
      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent className="flex max-h-[80vh] w-full max-w-lg flex-col overflow-hidden p-0">
          <DialogHeader className="border-b border-border/70 px-4 py-3">
            <DialogTitle className="text-sm font-medium">Files</DialogTitle>
            <DialogDescription className="sr-only">
              Every file shared in this channel. Files superseded by a newer
              upload are marked outdated.
            </DialogDescription>
          </DialogHeader>
          <div className="min-h-0 flex-1 overflow-y-auto p-3">
            {filesQuery.isPending ? (
              <div className="flex justify-center py-8 text-sm text-muted-foreground">
                Loading files…
              </div>
            ) : filesQuery.isError ? (
              <div className="flex flex-col items-center gap-2 py-8 text-center text-sm text-muted-foreground">
                <AlertCircle className="h-4 w-4" />
                Couldn't load this channel's files.
              </div>
            ) : chains.length > 0 ? (
              <div className="flex flex-col gap-1.5">
                {chains.map((chain) => (
                  <FileChainRow
                    chain={chain}
                    // A single message can carry more than one `imeta`
                    // attachment, so eventId alone isn't unique here.
                    key={`${chain.latest.eventId}-${
                      chain.latest.sha256 ??
                      chain.latest.url ??
                      chain.latest.filename
                    }`}
                    onOpen={handleOpenFile}
                  />
                ))}
              </div>
            ) : (
              <div className="flex justify-center py-8 text-sm text-muted-foreground">
                No files have been shared in this channel yet.
              </div>
            )}
          </div>
        </DialogContent>
      </Dialog>
      {previewFile && previewHref && previewKind ? (
        <FilePreviewModal
          href={previewHref}
          filename={previewFile.filename ?? "file"}
          open={previewFile !== null}
          onOpenChange={(next) => {
            if (!next) setPreviewFile(null);
          }}
          previewKind={previewKind}
          size={previewFile.size ?? undefined}
          versionStatus={fileVersionStatus(previewFile)}
        />
      ) : null}
    </>
  );
}
