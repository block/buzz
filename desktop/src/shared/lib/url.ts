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

/**
 * Editor deep-link schemes that Buzz may render and hand to the OS opener.
 *
 * Keep this list tight: custom schemes bypass the browser and invoke whatever
 * app registered the protocol. Do not add schemes that can execute code
 * (`javascript:`) or that lack a clear local-app owner.
 */
export const EDITOR_DEEP_LINK_PROTOCOLS = ["cursor:", "vscode:"] as const;

export type EditorDeepLinkProtocol =
  (typeof EDITOR_DEEP_LINK_PROTOCOLS)[number];

/**
 * TipTap Link `protocols` entries (scheme without trailing colon).
 * http(s) and mailto are accepted by TipTap by default.
 */
export const EDITOR_DEEP_LINK_TIPTAP_PROTOCOLS = EDITOR_DEEP_LINK_PROTOCOLS.map(
  (protocol) => protocol.slice(0, -1),
);

/**
 * True when `url` is a cursor:// or vscode:// deep link the OS can hand off
 * to the registered editor. Used by the markdown renderer and click path so
 * react-markdown's defaultUrlTransform does not strip the href before open.
 */
export function isEditorDeepLink(url: string | undefined): url is string {
  if (!url) return false;
  try {
    const parsed = new URL(url);
    return (EDITOR_DEEP_LINK_PROTOCOLS as readonly string[]).includes(
      parsed.protocol,
    );
  } catch {
    return false;
  }
}
