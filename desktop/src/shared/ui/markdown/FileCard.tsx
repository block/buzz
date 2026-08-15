import * as React from "react";
import { ChevronRight, Download, FileText } from "lucide-react";
import { toast } from "sonner";

import { invokeTauri } from "@/shared/api/tauri";
import {
  useFileVersionInfo,
  useFileVersionJump,
} from "@/shared/context/FileVersionContext";
import { FilePreviewModal } from "@/shared/ui/filePreview/FilePreviewModal";
import { FileVersionBadge } from "@/shared/ui/FileVersionBadge";
import { cn } from "@/shared/lib/cn";
import { formatItemTimestamp } from "@/shared/lib/datetime";
import type { FilePreviewKind } from "@/shared/ui/markdownFileCard";
import { useSmoothCorners } from "@/shared/ui/smoothCorners";

/** Human-readable byte size: "820 B", "12.4 KB", "3.1 MB". */
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

/**
 * File card for a generic (non-image, non-video) attachment: icon, filename,
 * size, and either a preview or download action.
 *
 * When `previewKind` is set (PDF, text/code, markdown, .docx, .xlsx, .pptx),
 * clicking the card opens an in-app preview modal instead of going straight
 * to disk — download is still available from inside that modal. Unrecognized
 * types keep the original download-on-click behavior.
 *
 * Downloads go through the native `download_file` Tauri command (HTTP inside
 * the app's tunnel + a save dialog), not a plain `<a download>` link. A bare
 * link navigates the webview to the blob URL, which escapes to the OS browser
 * and gets bounced to a corporate CDN interstitial ("browser not supported").
 * The native command mirrors the image-download path.
 */
export function FileCard({
  href,
  filename,
  previewKind = null,
  size,
}: {
  href: string;
  filename: string;
  previewKind?: FilePreviewKind;
  size?: number;
}) {
  const cardRef = React.useRef<HTMLButtonElement | null>(null);
  const [isPreviewOpen, setIsPreviewOpen] = React.useState(false);
  const [isHistoryOpen, setIsHistoryOpen] = React.useState(false);
  const sizeLabel = size != null ? formatFileSize(size) : "";
  // `null` outside a channel (thread pane, Inbox preview, forum) — those
  // surfaces render the card without any version affordance, as before.
  const versionInfo = useFileVersionInfo(href);
  const jumpToMessage = useFileVersionJump();
  useSmoothCorners(cardRef);

  const latestEventId = versionInfo?.latestEventId ?? null;
  const handleJumpToLatest =
    jumpToMessage && latestEventId ? () => jumpToMessage(latestEventId) : null;

  // Only the head of a chain offers history. An outdated file gets the jump
  // affordance instead — rendering both would show the same chain nested
  // inside itself and put two competing actions on one card.
  const olderVersions = versionInfo?.olderVersions ?? [];
  const showHistory = olderVersions.length > 0 && !versionInfo?.status.outdated;

  const handleClick = () => {
    if (previewKind) {
      setIsPreviewOpen(true);
      return;
    }
    invokeTauri("download_file", { url: href, filename }).catch(
      (err: unknown) => {
        const msg = err instanceof Error ? err.message : "Download failed";
        toast.error(msg);
      },
    );
  };

  return (
    <span className="my-1 block max-w-sm">
      <button
        ref={cardRef}
        type="button"
        onClick={handleClick}
        data-testid="file-card"
        className="inline-flex w-full items-center gap-3 rounded-2xl border border-border/70 bg-muted/40 px-3 py-2 text-left no-underline transition-colors hover:bg-muted/70"
        style={{ borderRadius: "1rem" }}
      >
        <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-background text-muted-foreground">
          <FileText className="h-4 w-4" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm font-medium text-foreground">
            {filename}
          </span>
          <span className="flex items-center gap-1.5">
            {sizeLabel ? (
              <span className="text-xs text-muted-foreground">{sizeLabel}</span>
            ) : null}
            {versionInfo && versionInfo.total > 1 ? (
              <span className="text-xs text-muted-foreground">
                Version {versionInfo.position} of {versionInfo.total}
              </span>
            ) : null}
            <FileVersionBadge
              onJumpToLatest={handleJumpToLatest}
              status={versionInfo?.status}
            />
          </span>
        </span>
        <Download className="h-4 w-4 shrink-0 text-muted-foreground" />
      </button>
      {showHistory ? (
        <span className="mt-1 block">
          <button
            aria-expanded={isHistoryOpen}
            className="flex items-center gap-1 rounded px-1.5 py-0.5 text-2xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
            data-testid="file-card-history-toggle"
            onClick={() => setIsHistoryOpen((value) => !value)}
            type="button"
          >
            <ChevronRight
              className={cn(
                "h-3 w-3 transition-transform",
                isHistoryOpen && "rotate-90",
              )}
            />
            {isHistoryOpen
              ? "Hide earlier versions"
              : `Supersedes ${olderVersions.length} earlier version${
                  olderVersions.length === 1 ? "" : "s"
                }`}
          </button>
          {isHistoryOpen ? (
            <span className="mt-1 flex flex-col gap-0.5 border-l border-border/60 pl-3">
              {olderVersions.map((older, index) => {
                const label = older.filename ?? "Untitled file";
                const position = versionInfo
                  ? versionInfo.total - 1 - index
                  : null;
                const canJump = Boolean(jumpToMessage);
                return (
                  <button
                    className="flex items-baseline gap-1.5 rounded px-1 py-0.5 text-left text-2xs text-muted-foreground transition-colors enabled:hover:bg-muted enabled:hover:text-foreground disabled:cursor-default"
                    disabled={!canJump}
                    key={older.eventId}
                    onClick={() => jumpToMessage?.(older.eventId)}
                    type="button"
                  >
                    {position != null ? <span>v{position}</span> : null}
                    <span className="truncate">{label}</span>
                    <span className="shrink-0 opacity-70">
                      {formatItemTimestamp(older.uploadedAt, {
                        withTime: true,
                      })}
                    </span>
                  </button>
                );
              })}
            </span>
          ) : null}
        </span>
      ) : null}
      {previewKind ? (
        <FilePreviewModal
          href={href}
          filename={filename}
          onOpenChange={setIsPreviewOpen}
          open={isPreviewOpen}
          previewKind={previewKind}
          size={size}
          versionStatus={versionInfo?.status}
        />
      ) : null}
    </span>
  );
}
