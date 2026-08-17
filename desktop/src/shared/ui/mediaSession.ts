import * as React from "react";

/**
 * macOS routes the hardware play/pause key (keyboard media keys, AirPods taps)
 * to whichever app most recently registered a Now Playing session. WKWebView
 * registers one for Buzz as soon as an audible `<video>` starts, and the claim
 * outlives removal from the DOM: a detached media element keeps its session
 * until it is garbage collected, so the key can keep reaching a player the
 * user scrolled past or closed minutes ago instead of going back to Music.
 *
 * Holding the keys while a video is on screen is correct — that is how resume
 * works in every macOS app. Handing them back when the video goes away is what
 * this module exists for: pause, detach the source (the only thing that makes
 * WebKit destroy the session immediately rather than at some later GC), and
 * clear the metadata this player published.
 */

export type NowPlayingMetadata = {
  /** Channel the video was posted in, shown as the Now Playing subtitle. */
  artist?: string;
  title: string;
};

/**
 * Element the current Now Playing metadata describes. Clearing is scoped to
 * the owner so an off-screen timeline row unmounting cannot wipe the metadata
 * of the review overlay the user is actually watching.
 */
let sessionOwner: HTMLVideoElement | null = null;

/** Sources detached by {@link releaseVideoElement}, for the StrictMode
 * re-attach path in {@link useReleasingVideoRef}. Weak so a released element
 * stays collectable. */
const detachedSources = new WeakMap<HTMLVideoElement, string>();

function getMediaSession(): MediaSession | null {
  // Feature-detected: WebKit has shipped the Media Session API since Safari
  // 15, but the unit-test environment's `navigator` has no `mediaSession`.
  if (typeof navigator === "undefined") {
    return null;
  }
  return navigator.mediaSession ?? null;
}

/** Publish what is playing so Control Center shows the video instead of a
 * bare "Buzz" entry, and record this element as the session's owner. */
export function claimMediaSession(
  video: HTMLVideoElement,
  metadata: NowPlayingMetadata,
): void {
  sessionOwner = video;
  const session = getMediaSession();
  if (!session) {
    return;
  }
  if (typeof MediaMetadata === "function") {
    session.metadata = new MediaMetadata({
      artist: metadata.artist ?? "",
      title: metadata.title,
    });
  }
  session.playbackState = "playing";
}

export function markMediaSessionPaused(video: HTMLVideoElement): void {
  const session = getMediaSession();
  if (!session || sessionOwner !== video) {
    return;
  }
  session.playbackState = "paused";
}

export function releaseMediaSession(video: HTMLVideoElement): void {
  if (sessionOwner !== video) {
    return;
  }
  sessionOwner = null;
  const session = getMediaSession();
  if (!session) {
    return;
  }
  session.metadata = null;
  session.playbackState = "none";
}

/**
 * Tear a media element down deterministically: stop playback, drop this
 * player's Now Playing metadata, and detach the source so WebKit destroys the
 * element's media session now instead of holding the media keys until the
 * detached element happens to be collected.
 */
export function releaseVideoElement(video: HTMLVideoElement): void {
  releaseMediaSession(video);
  video.pause();
  const src = video.getAttribute("src");
  if (src !== null) {
    detachedSources.set(video, src);
    video.removeAttribute("src");
  }
  video.load();
}

/**
 * Ref callback that releases its `<video>` when React drops it.
 *
 * Teardown cannot live in an effect cleanup: React detaches host refs during
 * the mutation phase, so on unmount a passive cleanup only ever sees a null
 * ref and the element escapes untouched. A ref cleanup still holds the
 * element, and runs before the node leaves the document.
 *
 * `onRelease` runs before the source is detached — that is the last moment a
 * caller can read `currentTime` off the element. Both options are re-read on
 * every render, so callers do not have to memoize them.
 */
export function useReleasingVideoRef(options?: {
  onRelease?: (video: HTMLVideoElement) => void;
  videoRef?: React.RefObject<HTMLVideoElement | null>;
}): (video: HTMLVideoElement) => () => void {
  const optionsRef = React.useRef(options);
  optionsRef.current = options;

  return React.useCallback((video: HTMLVideoElement) => {
    // StrictMode simulates a remount by running this cleanup and re-attaching
    // the same element — without re-applying props. Restore a source the
    // release detached, or every video renders empty in development.
    const detachedSrc = detachedSources.get(video);
    if (detachedSrc !== undefined) {
      detachedSources.delete(video);
      if (!video.hasAttribute("src")) {
        video.setAttribute("src", detachedSrc);
        video.load();
      }
    }
    const attachedRef = optionsRef.current?.videoRef;
    if (attachedRef) {
      attachedRef.current = video;
    }

    return () => {
      const current = optionsRef.current;
      current?.onRelease?.(video);
      if (current?.videoRef) {
        current.videoRef.current = null;
      }
      releaseVideoElement(video);
    };
  }, []);
}
