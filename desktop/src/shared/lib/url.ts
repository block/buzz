/**
 * Returns true when the given string is a valid HTTP or HTTPS URL.
 *
 * Use this to gate any user-supplied URL before rendering it as an `<a href>`,
 * opening it in a browser, or embedding it in an iframe.
 */
export function isSafeUrl(url: string | undefined): url is string {
  if (!url) return false;
  try {
    const parsed = new URL(url);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
  }
}
