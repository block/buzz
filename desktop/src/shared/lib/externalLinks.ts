// Web (and especially the installed PWA): external http(s) links must open in the system browser, never navigate
// the app itself. A capture-phase guard marks any external anchor target=_blank rel=noopener noreferrer just before
// its click is handled, so the browser opens it outside the app. In-app (same-origin) routes are left alone, and
// handlers that preventDefault and call openUrl keep working (no double open).

export function isExternalHttpUrl(href: string | null | undefined, origin: string): boolean {
  if (!href) return false;
  try {
    const u = new URL(href, origin);
    return (u.protocol === "http:" || u.protocol === "https:") && u.origin !== origin;
  } catch {
    return false;
  }
}

export function hardenExternalAnchor(a: HTMLAnchorElement, origin: string): boolean {
  if (!isExternalHttpUrl(a.getAttribute("href"), origin)) return false;
  a.target = "_blank";
  const rel = new Set((a.getAttribute("rel") ?? "").split(/\s+/).filter(Boolean));
  rel.add("noopener");
  rel.add("noreferrer");
  a.setAttribute("rel", [...rel].join(" "));
  return true;
}

export function installExternalLinkGuard(doc: Document = document): void {
  const origin = doc.defaultView?.location.origin ?? "";
  const onClick = (event: Event) => {
    const a = (event.target as Element | null)?.closest?.("a[href]") as HTMLAnchorElement | null;
    if (a) hardenExternalAnchor(a, origin);
  };
  doc.addEventListener("click", onClick, true);
  doc.addEventListener("auxclick", onClick, true);
}
