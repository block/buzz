/** Web/PWA document title. Desktop Tauri keeps productName ("Hula Buzz" / "Buzz"). */

export const WEB_APP_TITLE = "HulaBuzz";

/** Set the tab / Add-to-Home-Screen title. Optional page segment → "#general · HulaBuzz". */
export function setWebDocumentTitle(page?: string | null): void {
  const segment = typeof page === "string" ? page.trim() : "";
  if (!segment || segment === WEB_APP_TITLE) {
    document.title = WEB_APP_TITLE;
    return;
  }
  // Avoid "HulaBuzz · HulaBuzz" if a caller already passed a full title.
  if (segment.endsWith(` · ${WEB_APP_TITLE}`) || segment.endsWith(` — ${WEB_APP_TITLE}`)) {
    document.title = segment;
    return;
  }
  document.title = `${segment} · ${WEB_APP_TITLE}`;
}
