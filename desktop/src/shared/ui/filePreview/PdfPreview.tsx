import * as React from "react";
import * as pdfjsLib from "pdfjs-dist";
// Vite `?url` import: bundles the worker as a same-origin asset so it satisfies
// the app's CSP (`script-src 'self'`) instead of pdf.js's CDN default.
import pdfWorkerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";

pdfjsLib.GlobalWorkerOptions.workerSrc = pdfWorkerUrl;

/** CSS pixels per PDF point at 100% zoom-equivalent rendering. */
const RENDER_SCALE = 1.5;

/**
 * Renders every page of a PDF as stacked, scrollable canvases.
 *
 * Pages render progressively (page N+1 starts once page N's canvas is
 * created) rather than all at once, so a long document starts showing content
 * immediately instead of blocking on the whole file.
 */
export function PdfPreview({ bytes }: { bytes: Uint8Array }) {
  const containerRef = React.useRef<HTMLDivElement | null>(null);
  const [pageCount, setPageCount] = React.useState<number | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let cancelled = false;
    let doc: pdfjsLib.PDFDocumentProxy | null = null;
    const container = containerRef.current;

    async function render() {
      try {
        // pdf.js transfers ownership of the buffer it's handed to a worker
        // thread; pass a copy so the caller's bytes are never detached.
        const loadingTask = pdfjsLib.getDocument({ data: bytes.slice() });
        doc = await loadingTask.promise;
        if (cancelled) return;
        setPageCount(doc.numPages);
        if (!container) return;

        for (let pageNumber = 1; pageNumber <= doc.numPages; pageNumber += 1) {
          if (cancelled) return;
          const page = await doc.getPage(pageNumber);
          if (cancelled) return;

          const viewport = page.getViewport({ scale: RENDER_SCALE });
          const canvas = document.createElement("canvas");
          canvas.width = viewport.width;
          canvas.height = viewport.height;
          canvas.className =
            "mx-auto mb-3 block max-w-full rounded-lg border border-border/60 shadow-sm";
          canvas.style.width = "100%";
          canvas.style.height = "auto";
          container.appendChild(canvas);

          const context = canvas.getContext("2d");
          if (!context) continue;
          await page.render({ canvasContext: context, viewport }).promise;
        }
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : "Failed to render PDF");
        }
      }
    }

    void render();

    return () => {
      cancelled = true;
      void doc?.destroy();
      if (container) container.innerHTML = "";
    };
  }, [bytes]);

  if (error) {
    return (
      <div className="flex h-full items-center justify-center p-8 text-center text-sm text-muted-foreground">
        {error}
      </div>
    );
  }

  return (
    <div className="p-4">
      {pageCount === null ? (
        <div className="flex justify-center py-8 text-sm text-muted-foreground">
          Loading PDF…
        </div>
      ) : null}
      <div ref={containerRef} />
    </div>
  );
}
