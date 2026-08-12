import * as React from "react";
import { Download, FileText } from "lucide-react";

import { downloadAttachment } from "@/features/fileViewer/downloadAttachment";
import { classifyFileView } from "@/features/fileViewer/fileViewClassification";
import {
  hasFileViewerHost,
  openFileViewerTab,
} from "@/features/fileViewer/fileViewerStore";
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
 * Card for a generic (non-image, non-video) attachment: icon, filename, size,
 * and a download action.
 *
 * Clicking a viewable file opens the file-viewer panel. Non-viewable types, and
 * surfaces with no mounted viewer host (e.g. forum routes), download instead.
 */
export function FileCard({
  href,
  filename,
  mime,
  size,
}: {
  href: string;
  filename: string;
  mime?: string;
  size?: number;
}) {
  const cardRef = React.useRef<HTMLButtonElement | null>(null);
  const sizeLabel = size != null ? formatFileSize(size) : "";
  useSmoothCorners(cardRef);
  const isViewable = classifyFileView(filename, mime).kind !== "none";

  return (
    <span className="relative my-1 inline-flex max-w-sm">
      <button
        ref={cardRef}
        type="button"
        onClick={() => {
          if (isViewable && hasFileViewerHost()) {
            openFileViewerTab({ filename, mime, size, url: href });
            return;
          }
          downloadAttachment(href, filename);
        }}
        data-testid="file-card"
        className="inline-flex w-full items-center gap-3 rounded-2xl border border-border/70 bg-muted/40 py-2 pl-3 pr-10 text-left no-underline transition-colors hover:bg-muted/70 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
        style={{ borderRadius: "1rem" }}
      >
        <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-background text-muted-foreground">
          <FileText className="h-4 w-4" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm font-medium text-foreground">
            {filename}
          </span>
          {sizeLabel ? (
            <span className="block text-xs text-muted-foreground">
              {sizeLabel}
            </span>
          ) : null}
        </span>
      </button>
      <button
        aria-label={`Download ${filename}`}
        className="absolute right-3 top-1/2 -translate-y-1/2 rounded-md p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
        data-testid="file-card-download"
        onClick={() => downloadAttachment(href, filename)}
        type="button"
      >
        <Download className="h-4 w-4" />
      </button>
    </span>
  );
}
