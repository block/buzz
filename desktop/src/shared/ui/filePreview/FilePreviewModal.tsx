import * as React from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { Download, X } from "lucide-react";
import { toast } from "sonner";

import { invokeTauri } from "@/shared/api/tauri";
import { fetchMediaBytes } from "@/shared/api/tauriMedia";
import type { FileVersionStatus } from "@/shared/context/FileVersionContext";
import { cn } from "@/shared/lib/cn";
import { FileVersionBadge } from "@/shared/ui/FileVersionBadge";
import {
  TEXT_PREVIEW_MAX_BYTES,
  type FilePreviewKind,
} from "@/shared/ui/markdownFileCard";
import { MODAL_BACKDROP_BLUR_CLASS } from "@/shared/ui/modalBackdrop";

import { DocxPreview } from "./DocxPreview";
import { PdfPreview } from "./PdfPreview";
import { PptxSmartPreview } from "./PptxSmartPreview";
import { TextFilePreview } from "./TextFilePreview";
import { XlsxPreview } from "./XlsxPreview";

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

function isTextLikeKind(kind: FilePreviewKind): boolean {
  return kind === "text" || kind === "markdown";
}

function downloadFile(href: string, filename: string): void {
  invokeTauri("download_file", { url: href, filename }).catch(
    (err: unknown) => {
      const msg = err instanceof Error ? err.message : "Download failed";
      toast.error(msg);
    },
  );
}

/**
 * Full-screen in-app preview for a file attachment: PDF, text/code,
 * markdown, .docx, .xlsx, or .pptx. Fetches bytes once via the same bounded,
 * SSRF-guarded `fetch_media_bytes` Tauri command the composer's image editor
 * already uses, then hands them to a type-specific renderer.
 *
 * Modeled on `SimpleImageLightbox` (same Radix Dialog + backdrop pattern),
 * generalized with a header bar (filename/size/download/close) since a
 * document preview needs more than an image's bare full-bleed content.
 */
export function FilePreviewModal({
  href,
  filename,
  onOpenChange,
  open,
  previewKind,
  size,
  versionStatus = null,
}: {
  href: string;
  filename: string;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  previewKind: FilePreviewKind;
  size?: number;
  /**
   * Passed in rather than read from `FileVersionContext` here, so this stays a
   * presentational component and each caller supplies the status it already
   * has: `FileCard` from the context, `FilesPanel` from its own files query.
   * Neither pays for a second fetch.
   */
  versionStatus?: FileVersionStatus | null;
}) {
  const [bytes, setBytes] = React.useState<Uint8Array | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [isLoading, setIsLoading] = React.useState(false);

  const tooLargeForTextPreview =
    isTextLikeKind(previewKind) &&
    size != null &&
    size > TEXT_PREVIEW_MAX_BYTES;

  React.useEffect(() => {
    if (!open) {
      return;
    }
    if (tooLargeForTextPreview) return;
    let cancelled = false;
    setError(null);
    setBytes(null);
    setIsLoading(true);

    fetchMediaBytes(href)
      .then((data) => {
        if (!cancelled) setBytes(data);
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : "Failed to load file");
        }
      })
      .finally(() => {
        if (!cancelled) setIsLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [open, href, tooLargeForTextPreview]);

  const handleDownload = React.useCallback(() => {
    downloadFile(href, filename);
  }, [href, filename]);

  const sizeLabel = size != null ? formatFileSize(size) : "";

  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay
          className={cn(
            "fixed inset-0 z-50 bg-black/80 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0",
            MODAL_BACKDROP_BLUR_CLASS,
          )}
        />
        <DialogPrimitive.Content
          className="fixed inset-0 z-50 flex items-center justify-center p-4 sm:p-8"
          onInteractOutside={(event) => event.preventDefault()}
        >
          <div className="flex h-full max-h-[90vh] w-full max-w-4xl flex-col overflow-hidden rounded-2xl border border-border/70 bg-background shadow-2xl">
            <div className="flex shrink-0 items-center gap-3 border-b border-border/70 px-4 py-3">
              <DialogPrimitive.Title className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">
                {filename}
              </DialogPrimitive.Title>
              <DialogPrimitive.Description className="sr-only">
                File preview. Press Escape or use the close button to dismiss.
              </DialogPrimitive.Description>
              <FileVersionBadge status={versionStatus} />
              {sizeLabel ? (
                <span className="shrink-0 text-xs text-muted-foreground">
                  {sizeLabel}
                </span>
              ) : null}
              <button
                aria-label="Download"
                className="shrink-0 rounded-full p-2 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                onClick={handleDownload}
                type="button"
              >
                <Download className="h-4 w-4" />
              </button>
              <DialogPrimitive.Close
                aria-label="Close preview"
                className="shrink-0 rounded-full p-2 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
              >
                <X className="h-4 w-4" />
              </DialogPrimitive.Close>
            </div>
            <div className="min-h-0 flex-1 overflow-auto">
              {tooLargeForTextPreview ? (
                <div className="flex h-full flex-col items-center justify-center gap-2 p-8 text-center">
                  <p className="text-sm text-muted-foreground">
                    This file is too large to preview inline.
                  </p>
                  <button
                    className="text-sm font-medium text-primary underline"
                    onClick={handleDownload}
                    type="button"
                  >
                    Download instead
                  </button>
                </div>
              ) : isLoading ? (
                <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
                  Loading preview…
                </div>
              ) : error ? (
                <div className="flex h-full flex-col items-center justify-center gap-2 p-8 text-center">
                  <p className="text-sm text-muted-foreground">{error}</p>
                  <button
                    className="text-sm font-medium text-primary underline"
                    onClick={handleDownload}
                    type="button"
                  >
                    Download instead
                  </button>
                </div>
              ) : bytes ? (
                <FilePreviewContent
                  bytes={bytes}
                  filename={filename}
                  kind={previewKind}
                />
              ) : null}
            </div>
          </div>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}

function FilePreviewContent({
  bytes,
  filename,
  kind,
}: {
  bytes: Uint8Array;
  filename: string;
  kind: FilePreviewKind;
}) {
  switch (kind) {
    case "pdf":
      return <PdfPreview bytes={bytes} />;
    case "markdown":
      return (
        <TextFilePreview bytes={bytes} filename={filename} mode="markdown" />
      );
    case "text":
      return <TextFilePreview bytes={bytes} filename={filename} mode="code" />;
    case "docx":
      return <DocxPreview bytes={bytes} />;
    case "xlsx":
      return <XlsxPreview bytes={bytes} />;
    case "pptx":
      return <PptxSmartPreview bytes={bytes} />;
    default:
      return null;
  }
}
