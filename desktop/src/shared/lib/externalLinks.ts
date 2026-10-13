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

/** iOS / iPadOS home-screen web app (where target=_blank opens an in-app Safari sheet, not Safari). */
export function isIosStandalone(win: Window = window): boolean {
  const nav = win.navigator as Navigator & { standalone?: boolean };
  const ios = /iP(hone|od|ad)/.test(nav.userAgent) || (nav.platform === "MacIntel" && nav.maxTouchPoints > 1);
  return ios && (nav.standalone === true || win.matchMedia?.("(display-mode: standalone)").matches === true);
}

/** `x-safari-https://…` hands the URL to the Safari app (iOS 17+). Only well-formed http(s), re-serialized by URL so
 *  spaces / unicode / stray characters are percent-encoded (Safari rejects them as "address is invalid"). */
export function safariSchemeUrl(url: string): string | null {
  if (!/^https?:\/\//i.test(url)) return null;
  try {
    return `x-safari-${new URL(url).href}`;
  } catch {
    return null;
  }
}

const BARE_HOST = /^(?:www\.)?[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)*\.[a-z]{2,}(?::\d+)?(?:[/?#].*)?$/i;

/**
 * Web: the URL a message link may open, or null when it can't open in a browser. Bare domains ("www.google.com",
 * "example.org/x") get https://; http(s), mailto and tel pass through (re-serialized); desktop-only or unsafe schemes
 * (file:, tauri:, asset:, buzz:, blob:, javascript:, data:, about:…) return null so callers render plain text instead
 * of handing iOS an address it rejects ("Safari cannot open the page because the address is invalid").
 */
export function normalizeExternalHref(href: string | null | undefined): string | null {
  const raw = href?.trim();
  if (!raw) return null;
  if (/^(mailto|tel):/i.test(raw)) return raw;
  if (/^https?:\/\//i.test(raw)) {
    try {
      return new URL(raw).href;
    } catch {
      return null;
    }
  }
  if (/^[a-z][a-z0-9+.-]*:/i.test(raw) && !/^[^:/]+:\d/.test(raw)) return null; // some other scheme
  if (raw.startsWith("//")) return normalizeExternalHref(`https:${raw}`);
  if (BARE_HOST.test(raw)) return normalizeExternalHref(`https://${raw}`);
  return null;
}

let lastOpen = { url: "", at: 0 };

/** iOS major version from the UA ("OS 18_1" / "Version/18.1"), or null. */
export function iosMajorVersion(ua: string): number | null {
  const m = ua.match(/OS (\d+)[_.]\d/) ?? ua.match(/Version\/(\d+)/);
  return m ? Number(m[1]) : null;
}

/**
 * Open an external URL outside the app. In an iOS 17+ installed PWA, hand it to the Safari app via
 * x-safari-https:// (undocumented Apple scheme; iOS may ask "Open in Safari?" once). Older iOS ignores that scheme,
 * so there we fall back to window.open, which opens the in-app Safari sheet. Elsewhere: window.open(_blank).
 */
export function openExternalUrl(input: string, win: Window = window): void {
  const url = normalizeExternalHref(input);
  if (!url) return; // nothing a browser can open (desktop-only scheme, malformed): never hand iOS an invalid address
  const now = Date.now();
  if (lastOpen.url === url && now - lastOpen.at < 1200) return; // de-dupe guard + handler double fires
  lastOpen = { url, at: now };
  const major = iosMajorVersion(win.navigator.userAgent);
  const safari = isIosStandalone(win) && (major === null || major >= 17) ? safariSchemeUrl(url) : null;
  if (safari) win.location.href = safari;
  else win.open(url, "_blank", "noopener,noreferrer");
}

export function installExternalLinkGuard(doc: Document = document): void {
  const win = doc.defaultView ?? window;
  const origin = win.location.origin ?? "";
  const onClick = (event: Event) => {
    const a = (event.target as Element | null)?.closest?.("a[href]") as HTMLAnchorElement | null;
    if (!a || !hardenExternalAnchor(a, origin)) return;
    // Web owns every plain external-link click: skip the desktop-only link side panel / pinned-site handlers
    // (stopImmediatePropagation at document capture runs before React's root listeners) and open outside the app.
    // iOS installed PWA hands the link URL itself (never location) to Safari; elsewhere a new tab.
    const m = event as MouseEvent;
    if (event.type === "click" && m.button === 0 && !m.metaKey && !m.ctrlKey && !m.shiftKey && !m.altKey) {
      event.preventDefault();
      event.stopImmediatePropagation();
      openExternalUrl(new URL(a.getAttribute("href") ?? "", origin).href, win);
    }
  };
  doc.addEventListener("click", onClick, true);
  doc.addEventListener("auxclick", onClick, true);
}
