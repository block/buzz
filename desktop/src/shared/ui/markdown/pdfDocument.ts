import type {
  PDFDocumentProxy,
  PDFPageProxy,
  RenderTask,
} from "pdfjs-dist/legacy/build/pdf.mjs";

import { fetchMediaBytes } from "@/shared/api/tauriMedia";
import { getCachedRelayOrigin } from "@/shared/lib/mediaUrl";
import { isRelayMediaHref, pdfRenderScale } from "./pdfPages";

type PdfJs = typeof import("pdfjs-dist/legacy/build/pdf.mjs");

/** Same ceiling the native `fetch_media_bytes` command enforces. */
const MAX_PDF_BYTES = 50 * 1024 * 1024;
/** Rendered first pages kept in memory (data URLs, a few hundred KB each). */
const MAX_CACHED_PREVIEWS = 40;

let pdfJsPromise: Promise<PdfJs> | null = null;

/**
 * Load pdf.js on first use so it stays out of the startup bundle (and out of
 * node unit tests that import the markdown renderer).
 *
 * The legacy build is deliberate: the modern build calls very new built-ins
 * (e.g. `Map.prototype.getOrInsertComputed`) natively, which older macOS
 * WKWebView and Linux WebKitGTK webviews lack. Vite emits the worker as a
 * same-origin asset, satisfying the `worker-src 'self'` CSP.
 */
function loadPdfJs(): Promise<PdfJs> {
  if (!pdfJsPromise) {
    pdfJsPromise = Promise.all([
      import("pdfjs-dist/legacy/build/pdf.mjs"),
      import("pdfjs-dist/legacy/build/pdf.worker.min.mjs?url"),
    ]).then(
      ([pdfjs, worker]) => {
        pdfjs.GlobalWorkerOptions.workerSrc = worker.default;
        return pdfjs;
      },
      (error: unknown) => {
        pdfJsPromise = null;
        throw error;
      },
    );
  }
  return pdfJsPromise;
}

async function fetchPdfBytes(
  href: string,
  signal?: AbortSignal,
): Promise<Uint8Array> {
  if (isRelayMediaHref(href, getCachedRelayOrigin())) {
    return fetchMediaBytes(href, signal);
  }
  const response = await fetch(href, { signal });
  if (!response.ok) throw new Error(`PDF fetch failed (${response.status})`);
  const declaredLength = Number(response.headers.get("content-length"));
  if (declaredLength > MAX_PDF_BYTES) throw new Error("PDF is too large");
  const bytes = new Uint8Array(await response.arrayBuffer());
  if (bytes.byteLength > MAX_PDF_BYTES) throw new Error("PDF is too large");
  return bytes;
}

/**
 * Fetch and parse the PDF at `href`. The caller owns the returned document
 * and must release it with `doc.loadingTask.destroy()`, which also tears
 * down its worker.
 */
export async function openPdfDocument(
  href: string,
  signal?: AbortSignal,
): Promise<PDFDocumentProxy> {
  const data = await fetchPdfBytes(href, signal);
  const pdfjs = await loadPdfJs();
  if (signal?.aborted) {
    throw new DOMException("PDF load cancelled", "AbortError");
  }
  const task = pdfjs.getDocument({ data, enableXfa: false });
  const onAbort = () => void task.destroy();
  signal?.addEventListener("abort", onAbort, { once: true });
  try {
    return await task.promise;
  } finally {
    signal?.removeEventListener("abort", onAbort);
  }
}

/**
 * Render `page` into `canvas` so it displays `cssWidth` CSS pixels wide.
 * Returns the pdf.js task so callers can cancel a superseded render.
 */
export function renderPdfPage(
  page: PDFPageProxy,
  canvas: HTMLCanvasElement,
  cssWidth: number,
): RenderTask {
  const base = page.getViewport({ scale: 1 });
  const viewport = page.getViewport({
    scale: pdfRenderScale(base.width, cssWidth, window.devicePixelRatio),
  });
  canvas.width = Math.max(1, Math.floor(viewport.width));
  canvas.height = Math.max(1, Math.floor(viewport.height));
  return page.render({ canvas, viewport });
}

/** A rendered first page: an image URL plus its height / width ratio. */
export type PdfPreview = { src: string; aspect: number };

const previewCache = new Map<string, Promise<PdfPreview>>();

async function renderFirstPagePreview(
  href: string,
  cssWidth: number,
): Promise<PdfPreview> {
  const doc = await openPdfDocument(href);
  try {
    const page = await doc.getPage(1);
    const canvas = document.createElement("canvas");
    await renderPdfPage(page, canvas, cssWidth).promise;
    return {
      src: canvas.toDataURL("image/png"),
      aspect: canvas.height / canvas.width,
    };
  } finally {
    void doc.loadingTask.destroy();
  }
}

/**
 * First-page preview for the PDF at `href`, rendered once per href and shared
 * by every card showing it (virtualized timelines remount cards constantly).
 * Failures are evicted so a later mount can retry; the cache is bounded and
 * cleared on community switch via `resetPdfPreviewCache`.
 */
export function loadPdfPreview(
  href: string,
  cssWidth: number,
): Promise<PdfPreview> {
  const cached = previewCache.get(href);
  if (cached) return cached;
  const pending = renderFirstPagePreview(href, cssWidth);
  previewCache.set(href, pending);
  if (previewCache.size > MAX_CACHED_PREVIEWS) {
    const oldest = previewCache.keys().next().value;
    if (oldest !== undefined) previewCache.delete(oldest);
  }
  pending.catch(() => {
    if (previewCache.get(href) === pending) previewCache.delete(href);
  });
  return pending;
}

/** Drop every cached first-page preview (community switch). */
export function resetPdfPreviewCache(): void {
  previewCache.clear();
}
