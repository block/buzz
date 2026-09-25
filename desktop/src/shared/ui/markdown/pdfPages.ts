/**
 * Pure layout and fetch-routing helpers for the PDF attachment preview and
 * viewer. Kept free of pdf.js and React so they are unit-testable in node.
 */

/** Aspect ratio (height / width) used before a page's real size is known. */
export const DEFAULT_PDF_PAGE_ASPECT = 1.294; // US Letter; A4 is 1.414

/** Pages rendered on each side of the current page in the viewer. */
export const PDF_VIEWER_RENDER_RADIUS = 2;

/** Device-pixel-ratio ceiling for rendered canvases (bounds canvas memory). */
const MAX_RENDER_PIXEL_RATIO = 2;

/**
 * Whether PDF bytes for `href` should come over the authenticated Tauri media
 * IPC (`fetch_media_bytes`) rather than a plain webview `fetch`.
 *
 * Relay media sits behind the VPN / Cloudflare Access and needs the Blossom
 * read token, which only the Rust side can mint. External Blossom hosts are
 * fetched directly (BUD-01 requires them to send permissive CORS headers).
 * Until the relay origin has been resolved, assume relay — the same safe
 * default `rewriteRelayUrl` uses.
 */
export function isRelayMediaHref(
  href: string,
  relayOrigin: string | null,
): boolean {
  let url: URL;
  try {
    url = new URL(href);
  } catch {
    return false;
  }
  if (!url.pathname.startsWith("/media/")) return false;
  return relayOrigin === null || url.origin === relayOrigin;
}

/**
 * Canvas scale that renders a page `pageWidth` PDF units wide at `cssWidth`
 * CSS pixels, sharpened for the display's pixel ratio (capped at 2x).
 */
export function pdfRenderScale(
  pageWidth: number,
  cssWidth: number,
  devicePixelRatio: number,
): number {
  if (!(pageWidth > 0) || !(cssWidth > 0)) return 1;
  const ratio = Math.min(
    Math.max(devicePixelRatio || 1, 1),
    MAX_RENDER_PIXEL_RATIO,
  );
  return (cssWidth / pageWidth) * ratio;
}

/**
 * 1-based page number the viewer is "on": the last page whose top edge has
 * crossed the upper third of the scroll viewport. `pageTops` are page offsets
 * relative to the scroll container's content, in page order.
 */
export function pdfPageAtScroll(
  pageTops: readonly number[],
  scrollTop: number,
  viewportHeight: number,
): number {
  if (pageTops.length === 0) return 1;
  const probe = scrollTop + viewportHeight / 3;
  let page = 1;
  for (let index = 0; index < pageTops.length; index += 1) {
    if (pageTops[index] <= probe) page = index + 1;
    else break;
  }
  return page;
}

/**
 * Whether 1-based `page` is close enough to `currentPage` to hold a rendered
 * canvas. Pages outside the window are released so a long document never
 * keeps hundreds of full-resolution canvases alive.
 */
export function isPdfPageInRenderWindow(
  page: number,
  currentPage: number,
  radius: number = PDF_VIEWER_RENDER_RADIUS,
): boolean {
  return Math.abs(page - currentPage) <= radius;
}

/** Page counter label, e.g. "3 / 17". */
export function pdfPageCounterLabel(page: number, pageCount: number): string {
  return `${Math.min(Math.max(page, 1), pageCount)} / ${pageCount}`;
}
