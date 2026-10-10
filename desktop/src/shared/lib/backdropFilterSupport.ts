import { invokeTauri } from "@/shared/api/tauri";

/**
 * Set on the root when the webview parses `backdrop-filter` but will not
 * paint it. `globals/utilities.css` makes the glass surfaces solid under it.
 */
export const BACKDROP_FILTER_UNPAINTED_ATTRIBUTE =
  "data-backdrop-filter-unpainted";

/**
 * Ask the native side whether `backdrop-filter` actually paints. WebKitGTK
 * without accelerated compositing passes `@supports (backdrop-filter: …)` yet
 * blurs nothing, so CSS alone cannot detect it.
 */
export async function initializeBackdropFilterSupport(): Promise<void> {
  const unpainted = await invokeTauri<boolean>(
    "get_backdrop_filter_unpainted",
  ).catch(() => false);
  if (unpainted) {
    document.documentElement.setAttribute(
      BACKDROP_FILTER_UNPAINTED_ATTRIBUTE,
      "",
    );
  }
}
