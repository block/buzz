export type ChannelWebsite = {
  id: string;
  title: string;
  url: string;
};

export type ChannelWebsitesDocument = {
  websites: ChannelWebsite[];
};

const MAX_WEBSITES = 12;
export const MAX_CHANNEL_WEBSITE_TITLE_LEN = 80;
const MAX_TITLE_LEN = MAX_CHANNEL_WEBSITE_TITLE_LEN;
const MAX_URL_LEN = 2048;

/** Chrome/Edge/Firefox error documents when X-Frame-Options / CSP blocks the iframe. */
export function isBlockedEmbedLocation(
  href: string | null | undefined,
): boolean {
  if (!href) return true;
  const lower = href.toLowerCase();
  return (
    lower === "about:blank" ||
    lower.startsWith("chrome-error:") ||
    lower.startsWith("chrome://") ||
    lower.startsWith("edge://") ||
    lower.startsWith("about:neterror")
  );
}

/** Returns true when the URL is safe to embed in an iframe src. */
export function isEmbeddableChannelWebsiteUrl(url: string): boolean {
  try {
    const parsed = new URL(url.trim());
    if (parsed.protocol !== "https:" && parsed.protocol !== "http:") {
      return false;
    }
    const host = parsed.hostname.toLowerCase();
    if (host === "localhost" || host === "127.0.0.1" || host === "[::1]") {
      return parsed.protocol === "http:" || parsed.protocol === "https:";
    }
    return parsed.protocol === "https:";
  } catch {
    return false;
  }
}

export function channelWebsiteTabLabel(website: ChannelWebsite): string {
  const title = website.title.trim();
  if (title) return title;
  try {
    return new URL(website.url).hostname;
  } catch {
    return "Website";
  }
}

/** Public favicon for the tab — no relay round-trip. */
export function channelWebsiteFaviconUrl(url: string): string | null {
  try {
    const host = new URL(url).hostname;
    if (!host) return null;
    return `https://www.google.com/s2/favicons?domain=${encodeURIComponent(host)}&sz=32`;
  } catch {
    return null;
  }
}

export function normalizeChannelWebsiteUrl(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const withScheme = /^[a-z][a-z0-9+.-]*:/i.test(trimmed)
    ? trimmed
    : `https://${trimmed}`;
  if (!isEmbeddableChannelWebsiteUrl(withScheme)) return null;
  if (withScheme.length > MAX_URL_LEN) return null;
  return withScheme;
}

export function parseChannelWebsitesContent(content: string): ChannelWebsite[] {
  const trimmed = content.trim();
  if (!trimmed) return [];
  try {
    const parsed = JSON.parse(trimmed) as Partial<ChannelWebsitesDocument>;
    if (!parsed || !Array.isArray(parsed.websites)) return [];
    const out: ChannelWebsite[] = [];
    const seen = new Set<string>();
    for (const entry of parsed.websites) {
      if (!entry || typeof entry !== "object") continue;
      const id = typeof entry.id === "string" ? entry.id.trim() : "";
      const title =
        typeof entry.title === "string"
          ? entry.title.trim().slice(0, MAX_TITLE_LEN)
          : "";
      const url =
        typeof entry.url === "string"
          ? normalizeChannelWebsiteUrl(entry.url)
          : null;
      if (!id || !url || seen.has(id)) continue;
      seen.add(id);
      out.push({ id, title, url });
      if (out.length >= MAX_WEBSITES) break;
    }
    return out;
  } catch {
    return [];
  }
}

export function serializeChannelWebsites(
  websites: readonly ChannelWebsite[],
): string {
  return JSON.stringify({ websites: [...websites] });
}

export function validateChannelWebsiteDraft(input: {
  title: string;
  url: string;
}): { title: string; url: string } | null {
  let rawUrl = input.url;
  let rawTitle = input.title;
  // URL typed in the label box (the two fields look similar when empty).
  if (!rawUrl.trim() && rawTitle.trim()) {
    const titleAsUrl = normalizeChannelWebsiteUrl(rawTitle);
    if (titleAsUrl) {
      rawUrl = rawTitle;
      rawTitle = "";
    }
  }
  const url = normalizeChannelWebsiteUrl(rawUrl);
  if (!url) return null;
  const title = rawTitle.trim().slice(0, MAX_TITLE_LEN);
  return { title, url };
}

/** Drop harness-style noise if it ever lands in stored JSON (defensive). */
export function sanitizeChannelWebsitesForDisplay(
  websites: readonly ChannelWebsite[],
): ChannelWebsite[] {
  return websites.filter(
    (site) =>
      site.title.trim().length > 0 &&
      isEmbeddableChannelWebsiteUrl(site.url) &&
      !/^thinking:/i.test(site.title) &&
      !site.url.includes("PRETTY_BIN"),
  );
}
