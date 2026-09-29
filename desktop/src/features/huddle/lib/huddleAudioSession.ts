/**
 * Who owns getUserMedia + AudioWorklet → push_audio_pcm.
 *
 * Companion-only product: the huddle room webview (`isRoom`) is frontmost while
 * the user speaks. Main-owned AudioContext is suspended by WKWebView when the
 * companion is focused, which starves STT (deaf captions / no agent wake) even
 * with resume polls + keepalive oscillators. Capture must live in the focused
 * companion webview.
 */

/** True when this webview should own mic capture and the STT worklet. */
export function shouldOwnHuddleAudioSession(isRoom: boolean): boolean {
  return isRoom;
}

/**
 * Main starts/joins the Rust session but must not open getUserMedia when the
 * companion will own capture. Companion claims media after it mounts.
 */
export function shouldSetupMediaOnHuddleStart(
  ownsAudioSession: boolean,
): boolean {
  return ownsAudioSession;
}

/**
 * Audio-owner unmount (companion remount / close / Strict Mode / zombie recreate)
 * must release mic but must NOT end the Rust session. Ending is Leave only —
 * otherwise Creating-phase early open races main's start and yields
 * `cannot confirm active: phase is Idle`.
 */
export function shouldEndRustSessionOnAudioOwnerUnmount(): boolean {
  return false;
}

/**
 * Companion (or any audio owner) should claim mic once Rust is live and local
 * capture is not already running.
 */
export function shouldClaimHuddleMedia(options: {
  ownsAudioSession: boolean;
  phase: string | undefined | null;
  ephemeralChannelId: string | null | undefined;
  alreadyConnected: boolean;
  claimInFlight: boolean;
}): boolean {
  if (!options.ownsAudioSession) return false;
  if (options.alreadyConnected || options.claimInFlight) return false;
  if (!options.ephemeralChannelId) return false;
  return options.phase === "connected" || options.phase === "active";
}
