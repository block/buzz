import * as React from "react";
import { Download, Loader2, X } from "lucide-react";
import type { PDFDocumentProxy } from "pdfjs-dist/legacy/build/pdf.mjs";

import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/shared/ui/dialog";
import { downloadFileAttachment } from "./FileCard";
import { openPdfDocument, renderPdfPage } from "./pdfDocument";
import {
  DEFAULT_PDF_PAGE_ASPECT,
  isPdfPageInRenderWindow,
  pdfPageAtScroll,
  pdfPageCounterLabel,
} from "./pdfPages";

type LoadState =
  | { status: "loading" }
  | { status: "ready"; doc: PDFDocumentProxy }
  | { status: "failed" };

/**
 * In-app PDF viewer: a scrollable column of every page, fitted to the dialog
 * width and rendered lazily around the current page, with a page counter,
 * Download, and Close (Esc).
 */
export function PdfViewerDialog({
  href,
  filename,
  open,
  onOpenChange,
}: {
  href: string;
  filename: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        data-testid="pdf-viewer-dialog"
        showCloseButton={false}
        className="flex h-[calc(100vh-4rem)] max-w-4xl flex-col gap-0 overflow-hidden p-0"
        // Keep clicks inside the portalled viewer from reaching the message
        // row that owns the card in the React tree.
        onClick={(event) => event.stopPropagation()}
      >
        <PdfViewerContent href={href} filename={filename} />
      </DialogContent>
    </Dialog>
  );
}

function PdfViewerContent({
  href,
  filename,
}: {
  href: string;
  filename: string;
}) {
  const [load, setLoad] = React.useState<LoadState>({ status: "loading" });
  const [currentPage, setCurrentPage] = React.useState(1);

  React.useEffect(() => {
    const controller = new AbortController();
    let doc: PDFDocumentProxy | null = null;
    setLoad({ status: "loading" });
    openPdfDocument(href, controller.signal).then(
      (loaded) => {
        doc = loaded;
        if (controller.signal.aborted) {
          void loaded.loadingTask.destroy();
          return;
        }
        setLoad({ status: "ready", doc: loaded });
      },
      () => {
        if (!controller.signal.aborted) setLoad({ status: "failed" });
      },
    );
    return () => {
      controller.abort();
      if (doc) void doc.loadingTask.destroy();
    };
  }, [href]);

  const pageCount = load.status === "ready" ? load.doc.numPages : 0;

  return (
    <>
      <div className="flex shrink-0 items-center gap-3 border-b border-border/70 py-2 pl-4 pr-2">
        <div className="min-w-0 flex-1">
          <DialogTitle className="truncate text-sm font-medium tracking-normal">
            {filename}
          </DialogTitle>
          <DialogDescription className="sr-only">
            PDF preview. Scroll to read every page; press Escape to close.
          </DialogDescription>
        </div>
        {pageCount > 0 ? (
          <span
            aria-live="polite"
            className="shrink-0 text-xs tabular-nums text-muted-foreground"
            data-testid="pdf-viewer-page-counter"
          >
            {pdfPageCounterLabel(currentPage, pageCount)}
          </span>
        ) : null}
        <Button
          aria-label={`Download ${filename}`}
          data-testid="pdf-viewer-download"
          onClick={() => downloadFileAttachment(href, filename)}
          size="icon"
          variant="ghost"
        >
          <Download />
        </Button>
        <DialogClose asChild>
          <Button aria-label="Close" size="icon" variant="ghost">
            <X />
          </Button>
        </DialogClose>
      </div>
      {load.status === "ready" ? (
        <PdfViewerPages
          doc={load.doc}
          currentPage={currentPage}
          onCurrentPageChange={setCurrentPage}
        />
      ) : (
        <div className="flex flex-1 items-center justify-center bg-muted/40 text-sm text-muted-foreground">
          {load.status === "loading" ? (
            <Loader2
              aria-label="Loading PDF"
              className="h-5 w-5 animate-spin"
            />
          ) : (
            "This PDF could not be displayed. You can still download it."
          )}
        </div>
      )}
    </>
  );
}

function PdfViewerPages({
  doc,
  currentPage,
  onCurrentPageChange,
}: {
  doc: PDFDocumentProxy;
  currentPage: number;
  onCurrentPageChange: (page: number) => void;
}) {
  const scrollRef = React.useRef<HTMLDivElement | null>(null);
  const [pageWidth, setPageWidth] = React.useState(0);
  const [aspects, setAspects] = React.useState<number[]>(() =>
    Array.from({ length: doc.numPages }, () => DEFAULT_PDF_PAGE_ASPECT),
  );

  // Fit pages to the scroll column's content width, tracking dialog resizes.
  React.useLayoutEffect(() => {
    const scroller = scrollRef.current;
    if (!scroller) return;
    scroller.focus({ preventScroll: true });
    const measure = () => {
      const style = getComputedStyle(scroller);
      const padding =
        Number.parseFloat(style.paddingLeft) +
        Number.parseFloat(style.paddingRight);
      setPageWidth(Math.max(0, Math.floor(scroller.clientWidth - padding)));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(scroller);
    return () => observer.disconnect();
  }, []);

  // Most documents use one page size, so seed every placeholder with page 1's
  // aspect; each page corrects its own slot once it renders.
  React.useEffect(() => {
    let cancelled = false;
    void doc
      .getPage(1)
      .then((page) => {
        if (cancelled) return;
        const base = page.getViewport({ scale: 1 });
        const aspect = base.height / base.width;
        setAspects((current) =>
          current.map((value) =>
            value === DEFAULT_PDF_PAGE_ASPECT ? aspect : value,
          ),
        );
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [doc]);

  const handleScroll = React.useCallback(() => {
    const scroller = scrollRef.current;
    if (!scroller) return;
    // The scroller is `relative`, so each page's offsetTop is already in
    // scroll-content coordinates.
    const tops = Array.from(
      scroller.children,
      (child) => (child as HTMLElement).offsetTop,
    );
    onCurrentPageChange(
      pdfPageAtScroll(tops, scroller.scrollTop, scroller.clientHeight),
    );
  }, [onCurrentPageChange]);

  const handleAspect = React.useCallback((index: number, aspect: number) => {
    setAspects((current) => {
      if (Math.abs(current[index] - aspect) < 0.001) return current;
      const next = current.slice();
      next[index] = aspect;
      return next;
    });
  }, []);

  return (
    <div
      ref={scrollRef}
      aria-label="PDF pages"
      className="relative flex min-h-0 flex-1 flex-col items-stretch gap-4 overflow-y-auto bg-muted/40 p-4 focus:outline-hidden"
      data-testid="pdf-viewer-pages"
      onScroll={handleScroll}
      role="document"
      // biome-ignore lint/a11y/noNoninteractiveTabindex: the scrollable page column must receive keyboard focus so arrow/Page keys scroll it
      tabIndex={0}
    >
      {aspects.map((aspect, index) => (
        <PdfViewerPage
          // Pages are positional and never reorder.
          // biome-ignore lint/suspicious/noArrayIndexKey: stable page index
          key={index}
          aspect={aspect}
          doc={doc}
          index={index}
          onAspect={handleAspect}
          render={
            pageWidth > 0 && isPdfPageInRenderWindow(index + 1, currentPage)
          }
          width={pageWidth}
        />
      ))}
    </div>
  );
}

const PdfViewerPage = React.memo(function PdfViewerPage({
  aspect,
  doc,
  index,
  onAspect,
  render,
  width,
}: {
  aspect: number;
  doc: PDFDocumentProxy;
  index: number;
  onAspect: (index: number, aspect: number) => void;
  render: boolean;
  width: number;
}) {
  const canvasRef = React.useRef<HTMLCanvasElement | null>(null);

  React.useEffect(() => {
    if (!render) return;
    let cancelled = false;
    let task: ReturnType<typeof renderPdfPage> | null = null;
    void doc
      .getPage(index + 1)
      .then((page) => {
        const canvas = canvasRef.current;
        if (cancelled || !canvas) return;
        const base = page.getViewport({ scale: 1 });
        onAspect(index, base.height / base.width);
        task = renderPdfPage(page, canvas, width);
        return task.promise;
      })
      // A cancelled or failed page keeps its placeholder; other pages still
      // render and the header keeps Download available.
      .catch(() => undefined);
    return () => {
      cancelled = true;
      task?.cancel();
    };
  }, [doc, index, onAspect, render, width]);

  return (
    <div
      aria-label={`Page ${index + 1}`}
      className="relative w-full shrink-0 overflow-hidden rounded-md bg-background shadow-sm"
      data-testid="pdf-viewer-page"
      role="img"
      style={{ aspectRatio: `1 / ${aspect}` }}
    >
      {render ? (
        <canvas ref={canvasRef} className="block h-full w-full" />
      ) : null}
    </div>
  );
});
