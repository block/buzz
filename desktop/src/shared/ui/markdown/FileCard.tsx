import * as React from "react";
import { Download, Eye, FileText } from "lucide-react";
import { toast } from "sonner";

import { invokeTauri } from "@/shared/api/tauri";
import { classifyAttachmentPreview } from "@/shared/ui/attachmentPreview";
import { Button } from "@/shared/ui/button";
import { useSmoothCorners } from "@/shared/ui/smoothCorners";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

import { FilePreviewDialog } from "./FilePreviewDialog";

/** Human-readable byte size: "820 B", "12.4 KB", "3.1 MB". */
export function formatFileSize(bytes: number): string {
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
 */
export function FileCard({
  href,
  filename,
  mimeType,
  renderMarkdown,
  size,
}: {
  href: string;
  filename: string;
  mimeType?: string;
  renderMarkdown: (content: string) => React.ReactNode;
  size?: number;
}) {
  const cardRef = React.useRef<HTMLDivElement | null>(null);
  const [previewOpen, setPreviewOpen] = React.useState(false);
  const sizeLabel = size != null ? formatFileSize(size) : "";
  const previewKind = React.useMemo(
    () => classifyAttachmentPreview(filename, mimeType, href),
    [filename, href, mimeType],
  );
  const canPreview = previewKind.kind !== "none";
  useSmoothCorners(cardRef);

  const download = React.useCallback(
    (event?: React.SyntheticEvent) => {
      event?.preventDefault();
      event?.stopPropagation();
      invokeTauri("download_file", { url: href, filename }).catch(
        (err: unknown) => {
          const msg = err instanceof Error ? err.message : "Download failed";
          toast.error(msg);
        },
      );
    },
    [filename, href],
  );

  return (
    <>
      <div
        ref={cardRef}
        data-testid="file-card"
        className="my-1 inline-flex max-w-sm items-stretch overflow-hidden rounded-2xl border border-border/70 bg-muted/40 text-left no-underline transition-colors hover:bg-muted/70"
        style={{ borderRadius: "1rem" }}
      >
        <button
          aria-label={
            canPreview ? `Preview ${filename}` : `Download ${filename}`
          }
          className="flex min-w-0 flex-1 items-center gap-3 px-3 py-2 text-left"
          data-testid={canPreview ? "file-preview-open" : undefined}
          onClick={(event) => {
            event.preventDefault();
            event.stopPropagation();
            if (canPreview) setPreviewOpen(true);
            else download();
          }}
          type="button"
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
          {canPreview ? (
            <Eye className="h-4 w-4 text-muted-foreground" />
          ) : null}
        </button>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              aria-label={`Download ${filename}`}
              className="h-auto w-10 shrink-0 rounded-none border-l border-border/60 text-muted-foreground"
              data-testid="file-download"
              onClick={download}
              size="icon"
              type="button"
              variant="ghost"
            >
              <Download />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Download</TooltipContent>
        </Tooltip>
      </div>
      {canPreview ? (
        <FilePreviewDialog
          filename={filename}
          href={href}
          kind={previewKind}
          onOpenChange={setPreviewOpen}
          open={previewOpen}
          renderMarkdown={renderMarkdown}
          size={size}
          sizeLabel={sizeLabel}
        />
      ) : null}
    </>
  );
}
