const OPEN_PREFIX = "/open/";

/** `buzz://` navigation hosts. Each opens a destination in the selected community. */
const NAVIGATION_HOSTS = new Set([
  "message",
  "channel",
  "repo",
  "project",
  "pr",
  "issue",
]);

/**
 * Maps `/open/<rest>` to `buzz://<rest>`, keeping path and query byte for
 * byte. Anything but a navigation link yields `null`, so this public page
 * never forwards invite, authorization or install links.
 */
export function buzzLinkFromOpenUrl(
  pathname: string,
  search: string,
): string | null {
  if (!pathname.startsWith(OPEN_PREFIX)) return null;
  const rest = pathname.slice(OPEN_PREFIX.length);
  const host = rest.split("/", 1)[0];
  if (!NAVIGATION_HOSTS.has(host)) return null;
  return `buzz://${rest}${search}`;
}
