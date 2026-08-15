import * as React from "react";
import { Download, FileText } from "lucide-react";
import { toast } from "sonner";

import { invokeTauri } from "@/shared/api/tauri";
import { FilePreviewModal } from "@/shared/ui/filePreview/FilePreviewModal";
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
  const sizeLabel = size != null ? formatFileSize(size) : "";
  useSmoothCorners(cardRef);

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
    <>
      <button
        ref={cardRef}
        type="button"
        onClick={handleClick}
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
        <Download className="h-4 w-4 shrink-0 text-muted-foreground" />
      </button>
      {previewKind ? (
        <FilePreviewModal
          href={href}
          filename={filename}
          onOpenChange={setIsPreviewOpen}
          open={isPreviewOpen}
          previewKind={previewKind}
          size={size}
        />
      ) : null}
    </>
  );
}
