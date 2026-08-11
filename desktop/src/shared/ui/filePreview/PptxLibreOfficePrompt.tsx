import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Download, RotateCw } from "lucide-react";
import { toast } from "sonner";

/** LibreOffice's official download page. */
const LIBREOFFICE_DOWNLOAD_URL =
  "https://www.libreoffice.org/download/download/";

/**
 * Shown in place of the `.pptx` preview when no working LibreOffice install
 * was found on this machine.
 *
 * LibreOffice conversion gives a pixel-accurate PDF preview (real PowerPoint
 * layout engine output); without it, the only option is the client-side JS
 * renderer, which is functional but noticeably lower fidelity (lost text
 * positioning/sizing, occasional missing content). This prompt explains the
 * tradeoff and lets the user either install LibreOffice, retry detection
 * after installing, or proceed with the lower-fidelity preview anyway.
 */
export function PptxLibreOfficePrompt({
  errorDetail,
  onRetry,
  onViewBasicPreview,
  retrying,
}: {
  /** Set when a LibreOffice conversion was attempted but failed at runtime
   * (as opposed to LibreOffice simply not being found). */
  errorDetail?: string | null;
  onRetry: () => void;
  onViewBasicPreview: () => void;
  retrying: boolean;
}) {
  const [detailsOpen, setDetailsOpen] = React.useState(false);

  const handleDownload = React.useCallback(() => {
    void openUrl(LIBREOFFICE_DOWNLOAD_URL).catch(() => {
      toast.error("Failed to open the download page");
    });
  }, []);

  return (
    <div className="flex h-full flex-col items-center justify-center gap-4 p-8 text-center">
      <div className="max-w-sm space-y-1.5">
        <p className="text-sm font-medium text-foreground">
          Install LibreOffice for accurate PowerPoint previews
        </p>
        <p className="text-sm text-muted-foreground">
          {errorDetail
            ? "The LibreOffice-based preview couldn't render this file."
            : "Buzz can render an exact, pixel-accurate preview of this presentation using a local LibreOffice install — the current in-app preview is a simplified approximation and can lose text positioning, sizing, or content."}
        </p>
        {errorDetail ? (
          <div className="pt-1">
            <button
              className="text-xs font-medium text-muted-foreground underline"
              onClick={() => setDetailsOpen((open) => !open)}
              type="button"
            >
              {detailsOpen ? "Hide details" : "Show details"}
            </button>
            {detailsOpen ? (
              <p className="mt-1 break-words text-left text-xs text-muted-foreground/80">
                {errorDetail}
              </p>
            ) : null}
          </div>
        ) : null}
      </div>

      <div className="flex flex-col items-center gap-2">
        <button
          className="inline-flex items-center gap-2 rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition-colors hover:bg-primary/90"
          onClick={handleDownload}
          type="button"
        >
          <Download className="h-4 w-4" />
          Download LibreOffice
        </button>
        <button
          className="inline-flex items-center gap-1.5 text-xs text-muted-foreground underline-offset-4 hover:underline disabled:opacity-60"
          disabled={retrying}
          onClick={onRetry}
          type="button"
        >
          <RotateCw className={retrying ? "h-3 w-3 animate-spin" : "h-3 w-3"} />
          {retrying ? "Checking…" : "Retry"}
        </button>
      </div>

      <button
        className="text-xs text-muted-foreground underline-offset-4 hover:underline"
        onClick={onViewBasicPreview}
        type="button"
      >
        View basic preview instead
      </button>
    </div>
  );
}
