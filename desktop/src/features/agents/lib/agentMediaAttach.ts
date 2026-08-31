/**
 * Attaching a session's remote tracks to a media element.
 *
 * Its own module so the rule can be tested without loading `livekit-client` or
 * a DOM: the attach functions are supplied by the caller, and this decides only
 * *where* they land.
 */

/** The attach callbacks `useAgentMediaRoom` exposes for a live session. */
export type SessionTrackAttachers = {
  attachAudio: ((element: HTMLMediaElement) => () => void) | null;
  attachVideo: ((element: HTMLVideoElement) => () => void) | null;
};

/**
 * Attach every available track to `element`, returning one detach for all.
 *
 * **One element for both tracks, deliberately.** LiveKit's `attach` merges a
 * track into whatever `MediaStream` the element already carries, so a single
 * element means one stream, one jitter buffer and one playback clock — which is
 * what holds the voice on the lips. Two elements play from two independent
 * clocks and drift audibly; that shipped once and was reported as bad lip-sync.
 *
 * It is also how the element becomes audible at all. LiveKit's
 * `attachToElement` sets `element.muted = mediaStream.getAudioTracks().length
 * === 0`, so a video-only stream is muted by the SDK no matter what the markup
 * says — which is why attaching video alone produced a silent avatar.
 */
export function attachSessionTracks(
  element: HTMLVideoElement,
  { attachAudio, attachVideo }: SessionTrackAttachers,
): () => void {
  const detach: Array<() => void> = [];
  // Video first, so the element's stream exists before the audio track joins
  // it and the SDK's mute heuristic sees both.
  if (attachVideo) detach.push(attachVideo(element));
  if (attachAudio) detach.push(attachAudio(element));
  return () => {
    for (const dispose of detach) dispose();
  };
}
