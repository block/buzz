const TAG_NAME = "link-preview";
const HIDE_VALUE = "hide";

export function normalizeSuppressedLinkPreviewUrls(values) {
  const normalized = [];
  const seen = new Set();
  for (const value of values ?? []) {
    if (typeof value !== "string") continue;
    try {
      const url = new URL(value);
      if (url.protocol !== "https:") continue;
      url.hash = "";
      const href = url.href;
      if (!seen.has(href)) {
        seen.add(href);
        normalized.push(href);
      }
    } catch {
      // Ignore malformed or relative suppression values from untrusted events.
    }
  }
  return normalized;
}

export function getLinkPreviewSuppression(tags) {
  let all = false;
  const urls = [];
  for (const tag of tags ?? []) {
    if (tag?.[0] !== TAG_NAME) continue;
    if (tag.length === 2 && tag[1] === "none") all = true;
    if (tag.length === 3 && tag[1] === HIDE_VALUE) urls.push(tag[2]);
  }
  return { all, urls: normalizeSuppressedLinkPreviewUrls(urls) };
}

export function buildLinkPreviewSuppressionTags(values) {
  return normalizeSuppressedLinkPreviewUrls(values).map((href) => [
    TAG_NAME,
    HIDE_VALUE,
    href,
  ]);
}
