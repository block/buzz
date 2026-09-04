import * as React from "react";
import { Download, FileText, PanelRight } from "lucide-react";
import { toast } from "sonner";

import { invokeTauri } from "@/shared/api/tauri";
import { useSmoothCorners } from "@/shared/ui/smoothCorners";

import {
  isMarkdownDocFilename,
  MAX_MARKDOWN_DOC_BYTES,
} from "./markdownDocFile";
import { useMarkdownDocViewer } from "./markdownDocViewerContext";

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
 * size, and a download action.
 *
 * Downloads go through the native `download_file` Tauri command (HTTP inside
 * the app's tunnel + a save dialog), not a plain `<a download>` link. A bare
 * link navigates the webview to the blob URL, which escapes to the OS browser
 * and gets bounced to a corporate CDN interstitial ("browser not supported").
 * The native command mirrors the image-download path.
 *
 * Markdown attachments (`.md`/`.markdown`/`.mdx` by imeta filename) open the
 * in-app markdown viewer panel instead when a hosting surface provides one.
 * The authenticated `fetch_markdown_doc_bytes` command owns the authoritative
 * relay-origin check and native 2 MiB cap; duplicating its origin check here
 * makes the affordance depend on asynchronous frontend cache timing.
 */
export function FileCard({
  href,
  filename,
  size,
}: {
  href: string;
  filename: string;
  size?: number;
}) {
  const cardRef = React.useRef<HTMLButtonElement | null>(null);
  const sizeLabel = size != null ? formatFileSize(size) : "";
  useSmoothCorners(cardRef);
  const openMarkdownDoc = useMarkdownDocViewer();
  const isMarkdownDoc = isMarkdownDocFilename(filename);
  const isWithinPreviewSize = size == null || size <= MAX_MARKDOWN_DOC_BYTES;
  const opensInViewer =
    openMarkdownDoc !== null && isMarkdownDoc && isWithinPreviewSize;
  const downloadReason = !isMarkdownDoc
    ? "not-markdown"
    : !isWithinPreviewSize
      ? "too-large"
      : openMarkdownDoc === null
        ? "viewer-unavailable"
        : undefined;

  return (
    <button
      ref={cardRef}
      type="button"
      onClick={() => {
        if (opensInViewer) {
          openMarkdownDoc({ url: href, filename, opener: cardRef.current });
          return;
        }
        invokeTauri("download_file", { url: href, filename }).catch(
          (err: unknown) => {
            const msg = err instanceof Error ? err.message : "Download failed";
            toast.error(msg);
          },
        );
      }}
      aria-label={opensInViewer ? `Open ${filename}` : `Download ${filename}`}
      // Focus-return anchor: when the viewer panel closes, focus comes back
      // to the card that opened this URL (the element itself may have been
      // unmounted and remounted in the narrow layout meanwhile).
      data-doc-url={opensInViewer ? href : undefined}
      data-download-reason={downloadReason}
      data-filename={filename}
      data-size={size}
      data-testid="file-card"
      className="my-1 inline-flex max-w-sm items-center gap-3 rounded-2xl border border-border/70 bg-muted/40 px-3 py-2 text-left no-underline transition-colors hover:bg-muted/70"
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
      {opensInViewer ? (
        <PanelRight className="h-4 w-4 shrink-0 text-muted-foreground" />
      ) : (
        <Download className="h-4 w-4 shrink-0 text-muted-foreground" />
      )}
    </button>
  );
}
