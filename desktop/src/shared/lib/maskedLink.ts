/**
 * A link is "masked" when its visible text is not the destination URL —
 * `[click here](https://example.com)` — so the reader can't tell where it
 * goes. Masked links get a hover tooltip showing the full destination
 * (anti-phishing, mirrors Slack). Bare pasted URLs and GFM autolinks are
 * not masked: their label already is the URL, modulo cosmetic differences
 * (omitted scheme, trailing slash, host case).
 *
 * Deliberately strict where it matters: scheme downgrades (label says
 * https, href is http), path/query case changes, and protocol-relative
 * hrefs all count as masked.
 */
export function isMaskedLink(label: string, href: string): boolean {
  const trimmedLabel = label.trim();
  if (!trimmedLabel) return false;

  const hrefUrl = parseExternal(href);
  if (!hrefUrl) return false;

  const labelUrl = parseExternal(trimmedLabel, { assumeScheme: true });
  if (!labelUrl) return true; // label isn't URL-shaped at all

  // A label with userinfo (`safe.com@evil.com`) visually leads with the
  // wrong host — always treat it as masked.
  if (labelUrl.username || labelUrl.password) return true;

  // If the label spells out a scheme, it must match the destination's —
  // otherwise [https://x](http://x) silently downgrades.
  const labelHasScheme = /^https?:\/\//i.test(trimmedLabel);
  if (labelHasScheme && labelUrl.protocol !== hrefUrl.protocol) return true;

  // Host comparison is case-insensitive (the URL parser lowercases it);
  // path/query/hash stay case-sensitive — many backends distinguish them.
  return (
    labelUrl.host !== hrefUrl.host ||
    normalizePath(labelUrl.pathname) !== normalizePath(hrefUrl.pathname) ||
    labelUrl.search !== hrefUrl.search ||
    labelUrl.hash !== hrefUrl.hash
  );
}

function parseExternal(
  value: string,
  { assumeScheme = false }: { assumeScheme?: boolean } = {},
): URL | null {
  let candidate = value;
  if (/^\/\//.test(candidate)) {
    candidate = `https:${candidate}`;
  } else if (assumeScheme && !/^https?:\/\//i.test(candidate)) {
    candidate = `https://${candidate}`;
  }
  if (!/^https?:\/\//i.test(candidate)) return null;
  try {
    return new URL(candidate);
  } catch {
    return null;
  }
}

function normalizePath(pathname: string): string {
  return pathname.length > 1 ? pathname.replace(/\/$/, "") : pathname;
}
