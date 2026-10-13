import { useSonner } from "sonner";

/**
 * Height reserved at the bottom of native-webview hosts (pinned sites, link
 * panel, playground) while any toast is on screen.
 *
 * Those pages are native WKWebViews drawn above the HTML, so a toast would
 * render behind them. Instead of parking the site (a long "loading" toast hid
 * it entirely), the host shrinks by this strip and the bottom-anchored toast
 * stack shows in the gap. The site keeps running; it only sees a resize.
 * Sized for one collapsed toast plus Sonner's default 24px offset; an
 * expanded (hovered) stack can still grow upward under the site.
 */
export const TOAST_STRIP_PX = 104;

export function useToastStripInset(): number {
  const { toasts } = useSonner();
  return toasts.length > 0 ? TOAST_STRIP_PX : 0;
}

/** Style for an `h-full` native-webview host that yields the toast strip. */
export function toastStripHostStyle(
  inset: number,
): { height: string } | undefined {
  return inset > 0 ? { height: `calc(100% - ${inset}px)` } : undefined;
}
