import { isLongFormNaddr } from "./nostrAddress";

const NOSTR_SCHEME_PATTERN = /^nostr:/i;

export function isAllowedComposerLink(
  url: string,
  defaultValidate: (url: string) => boolean,
): boolean {
  return NOSTR_SCHEME_PATTERN.test(url)
    ? isLongFormNaddr(url)
    : defaultValidate(url);
}

export function shouldAutoLinkComposerUrl(
  url: string,
  defaultShouldAutoLink: (url: string) => boolean,
): boolean {
  return NOSTR_SCHEME_PATTERN.test(url)
    ? isLongFormNaddr(url)
    : defaultShouldAutoLink(url);
}
