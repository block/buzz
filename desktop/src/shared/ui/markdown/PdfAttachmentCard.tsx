import * as React from "react";
import { Download } from "lucide-react";

import { useSmoothCorners } from "@/shared/ui/smoothCorners";
import { downloadFileAttachment, FileCard, formatFileSize } from "./FileCard";
import { loadPdfPreview, type PdfPreview } from "./pdfDocument";
import { DEFAULT_PDF_PAGE_ASPECT } from "./pdfPages";
import { PdfViewerDialog } from "./PdfViewerDialog";

/** Rendered width of the first-page preview: the card's `max-w-sm` (24rem). */
const PREVIEW_CSS_WIDTH = 384;

type PreviewState =
  | { status: "loading" }
  | { status: "ready"; preview: PdfPreview }
  | { status: "failed" };

/**
 * Attachment card for a PDF: a first-page preview that opens the in-app
 * viewer, plus filename, size, and the same native Download action as
 * `FileCard`. While pdf.js renders, the imeta `image`/`thumb` (if any) or a
 * neutral placeholder holds the card's size; if rendering fails the card
 * degrades to the plain `FileCard` so the attachment is never blank.
 */
export function PdfAttachmentCard({
  href,
  filename,
  size,
  preview: posterSrc,
}: {
  href: string;
  filename: string;
  size?: number;
  preview?: string;
}) {
  const cardRef = React.useRef<HTMLSpanElement | null>(null);
  const [state, setState] = React.useState<PreviewState>({
    status: "loading",
  });
  const [viewerOpen, setViewerOpen] = React.useState(false);
  useSmoothCorners(cardRef);

  React.useEffect(() => {
    let active = true;
    setState({ status: "loading" });
    loadPdfPreview(href, PREVIEW_CSS_WIDTH).then(
      (preview) => {
        if (active) setState({ status: "ready", preview });
      },
      () => {
        if (active) setState({ status: "failed" });
      },
    );
    return () => {
      active = false;
    };
  }, [href]);

  if (state.status === "failed") {
    return <FileCard href={href} filename={filename} size={size} />;
  }

  const sizeLabel = size != null ? formatFileSize(size) : "";
  const imageSrc = state.status === "ready" ? state.preview.src : posterSrc;
  const aspect =
    state.status === "ready" ? state.preview.aspect : DEFAULT_PDF_PAGE_ASPECT;

  return (
    <span
      ref={cardRef}
      data-testid="pdf-attachment-card"
      data-pdf-preview-state={state.status}
      className="relative my-1 block w-full max-w-sm overflow-hidden rounded-2xl border border-border/70 bg-muted/40"
      style={{ borderRadius: "1rem" }}
    >
      <button
        type="button"
        aria-label={`Open ${filename}`}
        data-testid="pdf-attachment-open"
        onClick={() => setViewerOpen(true)}
        className="block w-full cursor-zoom-in text-left transition-colors hover:bg-muted/70 focus:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/50"
      >
        <span
          className="relative block max-h-72 w-full overflow-hidden border-b border-border/70 bg-background"
          style={{ aspectRatio: `1 / ${aspect}` }}
        >
          {imageSrc ? (
            <img
              alt=""
              src={imageSrc}
              draggable={false}
              className="block h-full w-full object-cover object-top"
            />
          ) : null}
          {state.status === "loading" ? (
            <span
              aria-hidden="true"
              className="absolute inset-0 animate-pulse bg-muted/50"
            />
          ) : null}
        </span>
        <span className="block min-w-0 py-2 pl-3 pr-12">
          <span className="block truncate text-sm font-medium text-foreground">
            {filename}
          </span>
          <span className="block text-xs text-muted-foreground">
            {sizeLabel ? `PDF · ${sizeLabel}` : "PDF"}
          </span>
        </span>
      </button>
      <button
        type="button"
        aria-label={`Download ${filename}`}
        data-testid="pdf-attachment-download"
        onClick={() => downloadFileAttachment(href, filename)}
        className="absolute bottom-2 right-2 flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus:outline-hidden focus-visible:ring-2 focus-visible:ring-ring/50"
      >
        <Download className="h-4 w-4" />
      </button>
      <PdfViewerDialog
        href={href}
        filename={filename}
        open={viewerOpen}
        onOpenChange={setViewerOpen}
      />
    </span>
  );
}
