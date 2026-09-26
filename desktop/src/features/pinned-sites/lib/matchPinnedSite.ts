import type { PinnedSite } from "./types";
import { normalizePinnedSiteUrl } from "./url";

function stripWww(hostname: string): string {
  return hostname.replace(/^www\./i, "").toLowerCase();
}

/**
 * Score how specifically `link` matches a pinned site URL.
 * - Path/prefix match on same host → score = matched prefix length (prefer longest)
 * - Same domain only → score = hostname length
 * - No match → -1
 */
export function scorePinnedSiteMatch(linkHref: string, pinUrl: string): number {
  let link: URL;
  let pin: URL;
  try {
    link = new URL(linkHref);
    const normalized = normalizePinnedSiteUrl(pinUrl);
    if (!normalized) return -1;
    pin = new URL(normalized);
  } catch {
    return -1;
  }
  if (link.protocol !== "http:" && link.protocol !== "https:") return -1;
  if (stripWww(link.hostname) !== stripWww(pin.hostname)) return -1;

  const linkPath = `${link.pathname.replace(/\/+$/, "") || ""}`;
  const pinPath = `${pin.pathname.replace(/\/+$/, "") || ""}`;
  const linkPrefix = `${stripWww(link.hostname)}${linkPath}${link.search}`;
  const pinPrefix = `${stripWww(pin.hostname)}${pinPath}${pin.search}`;

  // Front URL portion: link path starts with pin path (pin is a prefix of link)
  // or link equals / is under pin origin+path.
  if (pinPath === "" || pinPath === "/") {
    // Domain-only pin — weaker than a path pin.
    return stripWww(pin.hostname).length;
  }
  if (linkPath === pinPath || linkPath.startsWith(`${pinPath}/`)) {
    return pinPrefix.length;
  }
  // Also accept when the clicked URL is a prefix of the pin (share front portion).
  if (pinPath.startsWith(`${linkPath}/`) || pinPath === linkPath) {
    return linkPrefix.length;
  }
  // Same host, no path relationship — still a domain match.
  return stripWww(pin.hostname).length;
}

/** Prefer the longest/most specific pinned-site match for a link href. */
export function matchPinnedSiteForUrl(
  href: string,
  pins: readonly PinnedSite[],
): PinnedSite | null {
  let best: PinnedSite | null = null;
  let bestScore = -1;
  for (const pin of pins) {
    // Per-pin setting: unchecked pins are not domain-routing destinations.
    if (pin.openMatchingLinks === false) continue;
    const score = scorePinnedSiteMatch(href, pin.url);
    if (score > bestScore) {
      bestScore = score;
      best = pin;
    }
  }
  return bestScore >= 0 ? best : null;
}
