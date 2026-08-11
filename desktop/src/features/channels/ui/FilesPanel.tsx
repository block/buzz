import * as React from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  AlertCircle,
  Download,
  File as FileIcon,
  History,
  Link2,
} from "lucide-react";

import {
  type ChannelFileEntry,
  isOutdatedFile,
  linkChannelFileVersions,
  listChannelFiles,
} from "@/shared/api/channelFiles";
import { FileVersionPicker } from "@/features/messages/ui/FileVersionPicker";
import { classifyFilePreviewKind } from "@/shared/ui/markdownFileCard";
import { FilePreviewModal } from "@/shared/ui/filePreview/FilePreviewModal";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
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
  channelId,
  file,
  onLinked,
  onOpen,
}: {
  channelId: string;
  file: ChannelFileEntry;
  /** Called after a retroactive link publishes successfully, so the caller
   * can invalidate the files query and refresh badges. */
  onLinked: () => void;
  onOpen: (file: ChannelFileEntry) => void;
}) {
  const outdated = isOutdatedFile(file);
  const filename = file.filename ?? "Untitled file";
  const [pickerOpen, setPickerOpen] = React.useState(false);
  const [linking, setLinking] = React.useState(false);

  const handlePick = React.useCallback(
    (target: ChannelFileEntry) => {
      setLinking(true);
      void linkChannelFileVersions(channelId, file.eventId, target.eventId)
        .then(() => onLinked())
        .finally(() => setLinking(false));
    },
    [channelId, file.eventId, onLinked],
  );

  return (
    <div
      className={cn(
        "group flex items-start gap-3 rounded-lg border border-border/60 px-3 py-2.5 transition-colors hover:bg-muted/50",
        outdated && "opacity-70",
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
            {outdated ? (
              <span
                className="flex shrink-0 items-center gap-1 rounded-full bg-amber-500/15 px-2 py-0.5 text-3xs font-medium text-amber-600 dark:text-amber-400"
                title="A newer version of this file was shared later in this channel"
              >
                <AlertCircle className="h-3 w-3" />
                Outdated
              </span>
            ) : file.supersedes ? (
              <span
                className="flex shrink-0 items-center gap-1 rounded-full bg-muted px-2 py-0.5 text-3xs font-medium text-muted-foreground"
                title="Tagged as a newer version of an earlier upload"
              >
                <History className="h-3 w-3" />
                New version
              </span>
            ) : null}
          </div>
          <div className="mt-0.5 flex items-center gap-2 text-xs text-muted-foreground">
            <span>{truncatePubkey(file.uploadedBy)}</span>
            <span>&middot;</span>
            <span>
              {new Date(file.uploadedAt * 1000).toLocaleDateString(undefined, {
                month: "short",
                day: "numeric",
              })}
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
      {!outdated ? (
        <FileVersionPicker
          channelId={channelId}
          exclude={{ eventId: file.eventId, sha256: file.sha256 }}
          onOpenChange={setPickerOpen}
          onSelect={handlePick}
          open={pickerOpen}
          trigger={
            <button
              aria-label="Link to another file"
              className="mt-0.5 shrink-0 rounded p-1 text-muted-foreground/60 opacity-0 transition-opacity hover:bg-muted hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100 disabled:pointer-events-none disabled:opacity-40"
              data-testid="file-row-link-versions"
              disabled={linking}
              onClick={(event) => event.stopPropagation()}
              title="Link to another file…"
              type="button"
            >
              <Link2 className="h-3.5 w-3.5" />
            </button>
          }
        />
      ) : null}
      <Download className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground/60" />
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
  const queryClient = useQueryClient();
  const filesQuery = useQuery({
    queryKey: ["channel-files", channelId],
    queryFn: () => listChannelFiles(channelId as string),
    enabled: open && channelId != null,
  });
  const [previewFile, setPreviewFile] = React.useState<ChannelFileEntry | null>(
    null,
  );

  const handleLinked = React.useCallback(() => {
    void queryClient.invalidateQueries({
      queryKey: ["channel-files", channelId],
    });
  }, [queryClient, channelId]);

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
            ) : filesQuery.data && filesQuery.data.length > 0 ? (
              <div className="flex flex-col gap-1.5">
                {filesQuery.data.map((file) => (
                  <FileRow
                    // A single message can carry more than one `imeta`
                    // attachment, so eventId alone isn't unique here.
                    key={`${file.eventId}-${file.sha256 ?? file.url ?? file.filename}`}
                    channelId={channelId as string}
                    file={file}
                    onLinked={handleLinked}
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
        />
      ) : null}
    </>
  );
}
