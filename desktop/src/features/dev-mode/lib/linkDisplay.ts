const MAX_DISPLAY_CHARS = 64;

/** Slack-style link text: no scheme, no trailing slash, tail-truncated. */
export function linkDisplayText(href: string): string {
  const stripped = href.replace(/^https?:\/\//, "").replace(/\/$/, "");
  return stripped.length > MAX_DISPLAY_CHARS
    ? `${stripped.slice(0, MAX_DISPLAY_CHARS - 1)}…`
    : stripped;
}
