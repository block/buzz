/**
 * Animated avatar URL scheme.
 *
 * An animated avatar is persisted in the kind-0 `picture` field as a single
 * string so it round-trips through any Nostr client:
 *
 *   <posterUrl>#buzz-anim=<encodeURIComponent(animationUrl)>
 *
 * The poster is a static image (the frame the user picked) and the fragment
 * carries the animation (an animated PNG) to play on hover. Clients that
 * don't understand the scheme load the whole string as an image URL — the
 * fragment is never sent over HTTP, so they simply render the poster.
 */

const ANIMATED_AVATAR_SEPARATOR = "#buzz-anim=";

export type AnimatedAvatarDescriptor = {
  /** Static image shown when the avatar is idle. */
  posterUrl: string;
  /** Animated image played while the avatar is hovered. */
  animationUrl: string;
};

export function buildAnimatedAvatarUrl(
  posterUrl: string,
  animationUrl: string,
): string {
  return `${posterUrl}${ANIMATED_AVATAR_SEPARATOR}${encodeURIComponent(animationUrl)}`;
}

export function parseAnimatedAvatarUrl(
  url: string | null | undefined,
): AnimatedAvatarDescriptor | null {
  if (!url) {
    return null;
  }

  const separatorIndex = url.indexOf(ANIMATED_AVATAR_SEPARATOR);
  if (separatorIndex <= 0) {
    return null;
  }

  const posterUrl = url.slice(0, separatorIndex);
  const encodedAnimationUrl = url.slice(
    separatorIndex + ANIMATED_AVATAR_SEPARATOR.length,
  );
  if (encodedAnimationUrl.length === 0) {
    return null;
  }

  let animationUrl: string;
  try {
    animationUrl = decodeURIComponent(encodedAnimationUrl);
  } catch {
    return null;
  }

  if (!isHttpUrl(posterUrl) || !isHttpUrl(animationUrl)) {
    return null;
  }

  return { animationUrl, posterUrl };
}

export function getAvatarSnapshotUrl(
  url: string | null | undefined,
): string | null {
  if (!url) {
    return null;
  }

  return parseAnimatedAvatarUrl(url)?.posterUrl ?? url;
}

function isHttpUrl(value: string): boolean {
  return /^https?:\/\//i.test(value);
}
