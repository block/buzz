/**
 * Hula desktop OS deep-link scheme (`com.huladesk.buzz`).
 *
 * In-app builders (`buildChannelLink`, `buildMessageLink`, entity links) emit
 * canonical `buzz://…` URLs. The OS register and Tauri deep-link handler for
 * this build accept `hulabuzz://…` (see `build_identity::deep_link_scheme`).
 * Rewrite before putting a link on the clipboard so another session can open it.
 */

export const CANONICAL_DEEP_LINK_SCHEME = "buzz";
export const APP_DEEP_LINK_SCHEME = "hulabuzz";

const CANONICAL_PREFIX = `${CANONICAL_DEEP_LINK_SCHEME}://`;
const APP_PREFIX = `${APP_DEEP_LINK_SCHEME}://`;

/** Rewrite a canonical `buzz://…` href to the OS-openable `hulabuzz://…` form. */
export function toAppDeepLink(href: string): string {
  if (href.startsWith(CANONICAL_PREFIX)) {
    return `${APP_PREFIX}${href.slice(CANONICAL_PREFIX.length)}`;
  }
  return href;
}
