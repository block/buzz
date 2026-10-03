import {
  KIND_JOB_ACCEPTED,
  KIND_JOB_ERROR,
  KIND_JOB_PROGRESS,
  KIND_JOB_RESULT,
} from "@/shared/constants/kinds";
import type { FeedItemCategory } from "@/shared/api/types";

export const SOUND_NAMES = [
  "bong",
  "boo",
  "dng",
  "doo",
  "doodone",
  "doong",
  "doop",
  "flirl",
  "flutter",
  "oh-no",
  "ping",
  "unison",
] as const;
export type SoundName = (typeof SOUND_NAMES)[number];

export const SOUND_SLOTS = [
  "dm",
  "mention",
  "thread_reply",
  "needs_action",
  "job_accepted",
  "job_progress",
  "job_result",
  "job_error",
] as const;
export type SoundSlot = (typeof SOUND_SLOTS)[number];

export const SLOT_LABELS: Record<SoundSlot, string> = {
  dm: "Direct messages",
  mention: "@Mentions",
  thread_reply: "Thread replies",
  needs_action: "Needs action",
  job_accepted: "Agent: job accepted",
  job_progress: "Agent: progress update",
  job_result: "Agent: job result",
  job_error: "Agent: job error",
};

// The agent job protocol (kinds 43001-43006) is defined and queryable but
// nothing emits the events yet — buzz-acp publishes plain stream messages.
// These slots stay wired (resolver, defaults, settings) but render disabled
// with a "coming soon" badge until an emitter exists.
export const COMING_SOON_SLOTS: ReadonlySet<SoundSlot> = new Set([
  "job_accepted",
  "job_progress",
  "job_result",
  "job_error",
]);

export const SLOT_DESCRIPTIONS: Record<SoundSlot, string> = {
  dm: "When someone messages you directly.",
  mention: "When someone tags you in a channel.",
  thread_reply: "When someone replies in a thread you follow or posted in.",
  needs_action: "When an approval or reminder is waiting on you.",
  job_accepted: "When an agent picks up a job.",
  job_progress: "While an agent works through a job.",
  job_result: "When an agent finishes a job.",
  job_error: "When an agent job fails.",
};

export const RECOMMENDED_SOUND_BY_SLOT: Record<SoundSlot, SoundName> = {
  dm: "unison",
  mention: "ping",
  thread_reply: "doop",
  needs_action: "doodone",
  job_accepted: "boo",
  job_progress: "dng",
  job_result: "unison",
  job_error: "oh-no",
};

export type SlotSounds = Record<SoundSlot, SoundName>;

export const DEFAULT_SLOT_SOUNDS: SlotSounds = {
  dm: "flutter",
  mention: "flutter",
  thread_reply: "flutter",
  needs_action: "flutter",
  job_accepted: "flutter",
  job_progress: "flutter",
  job_result: "flutter",
  job_error: "flutter",
};

/** Per-event alerts (notification + sound) on/off. */
export const DEFAULT_SLOT_ALERTS_ENABLED: Record<SoundSlot, boolean> = {
  dm: true,
  mention: true,
  thread_reply: true,
  needs_action: true,
  job_accepted: true,
  job_progress: false,
  job_result: true,
  job_error: true,
};

export type SoundPreferences = {
  sounds: SlotSounds;
};

export function resolveSlotSound(
  prefs: SoundPreferences,
  slot: SoundSlot,
): SoundName {
  return prefs.sounds[slot];
}

/**
 * Pick the sound slot for a home-feed item.
 *
 * `category` is the backend's per-item classification (`FeedItemCategory` in
 * `desktop/src-tauri/src/models.rs`). A mention always wins — being addressed
 * directly outranks whatever kind of event carried it, including agent job
 * events. Every other known category maps explicitly; anything else falls
 * back to `needs_action` so a contract drift costs the user the wrong sound
 * rather than a missed alert — and warns so the drift is visible to
 * developers instead of silently masquerading as intended.
 */
export function slotForFeedKind(
  kind: number,
  category: FeedItemCategory,
): SoundSlot {
  if (category === "mention") return "mention";
  if (kind === KIND_JOB_ACCEPTED) return "job_accepted";
  if (kind === KIND_JOB_PROGRESS) return "job_progress";
  if (kind === KIND_JOB_RESULT) return "job_result";
  if (kind === KIND_JOB_ERROR) return "job_error";

  switch (category) {
    case "needs_action":
    case "activity":
    case "agent_activity":
      return "needs_action";
    default: {
      const unexpected: never = category;
      console.warn(
        `[notifications] unknown feed item category ${JSON.stringify(unexpected)} for kind ${kind}; falling back to needs_action`,
      );
      return "needs_action";
    }
  }
}

export function shouldPlayNotificationSound(
  channelId: string | null | undefined,
  silentChannelIds?: ReadonlySet<string>,
): boolean {
  return !channelId || !silentChannelIds?.has(channelId);
}

/**
 * A cancellable handle for an in-flight notification sound.
 *
 * `stop()` halts playback; `onEnded()` registers a callback fired once when
 * playback finishes naturally or is stopped (used by the settings preview
 * button to reset its play/pause state).
 */
export type SoundPlayback = {
  stop: () => void;
  onEnded: (callback: () => void) => void;
};

function soundUrl(name: SoundName): string {
  return `/sounds/${name}.mp3`;
}

// Notification sounds play through the Web Audio API rather than an
// <audio>/HTMLAudioElement. A playing HTMLMediaElement is automatically
// adopted by the browser's Media Session, which binds the OS hardware
// media keys (macOS Play/Pause) to it — so previewing a sound left the
// Play key replaying the ping indefinitely. Web Audio buffer sources are
// not media-session participants and are never captured by media keys.
// (This mirrors the click-poof sound in PoofBurstProvider.tsx.)
let audioContext: AudioContext | null = null;

function getAudioContext(): AudioContext | null {
  try {
    audioContext ??= new AudioContext({ latencyHint: "interactive" });
    return audioContext;
  } catch {
    return null;
  }
}

const bufferCache = new Map<SoundName, AudioBuffer>();
const bufferPromises = new Map<SoundName, Promise<AudioBuffer | null>>();

function loadBuffer(name: SoundName): Promise<AudioBuffer | null> {
  const cached = bufferCache.get(name);
  if (cached) {
    return Promise.resolve(cached);
  }
  const pending = bufferPromises.get(name);
  if (pending) {
    return pending;
  }

  const context = getAudioContext();
  if (!context) {
    return Promise.resolve(null);
  }

  const promise = fetch(soundUrl(name))
    .then((response) => {
      if (!response.ok) {
        throw new Error(`Failed to load sound ${name}: ${response.status}`);
      }
      return response.arrayBuffer();
    })
    .then((arrayBuffer) => context.decodeAudioData(arrayBuffer))
    .then((buffer) => {
      bufferCache.set(name, buffer);
      return buffer;
    })
    .catch(() => null)
    .finally(() => {
      bufferPromises.delete(name);
    });

  bufferPromises.set(name, promise);
  return promise;
}

/** Warm the decoded-buffer cache so the first play has no fetch/decode lag. */
export function preloadNotificationSound(name: SoundName): void {
  void loadBuffer(name);
}

function noopPlayback(): SoundPlayback {
  return { stop: () => {}, onEnded: (callback) => callback() };
}

// Fallback for environments without a usable AudioContext (or a buffer that
// failed to decode). Uses a one-shot HTMLAudioElement; this path can still be
// captured by media keys, but it only runs when Web Audio is unavailable.
function playViaAudioElement(name: SoundName): SoundPlayback {
  try {
    const audio = new Audio(soundUrl(name));
    audio.currentTime = 0;
    audio.play().catch(() => {
      // Best-effort — the user may not have interacted with the page yet.
    });
    return {
      stop: () => {
        audio.pause();
      },
      onEnded: (callback) => {
        const handler = () => callback();
        audio.addEventListener("ended", handler, { once: true });
        audio.addEventListener("pause", handler, { once: true });
      },
    };
  } catch {
    return noopPlayback();
  }
}

function playViaWebAudio(
  context: AudioContext,
  buffer: AudioBuffer,
): SoundPlayback {
  const endedCallbacks: Array<() => void> = [];
  let ended = false;
  const finish = () => {
    if (ended) return;
    ended = true;
    for (const callback of endedCallbacks) {
      callback();
    }
  };

  const source = context.createBufferSource();
  source.buffer = buffer;
  source.connect(context.destination);
  source.addEventListener("ended", finish, { once: true });

  const start = () => {
    try {
      source.start();
    } catch {
      finish();
    }
  };
  if (context.state === "suspended") {
    void context.resume().then(start, finish);
  } else {
    start();
  }

  return {
    stop: () => {
      try {
        source.stop();
      } catch {
        // Already stopped/ended.
      }
      finish();
    },
    onEnded: (callback) => {
      if (ended) {
        callback();
      } else {
        endedCallbacks.push(callback);
      }
    },
  };
}

export function playNotificationSound(name: SoundName): SoundPlayback {
  const context = getAudioContext();
  const buffer = bufferCache.get(name);

  if (context && buffer) {
    try {
      return playViaWebAudio(context, buffer);
    } catch {
      return playViaAudioElement(name);
    }
  }

  // Buffer not ready yet: warm the cache for next time and fall back to an
  // HTMLAudioElement for this play so the sound is not silently dropped.
  void loadBuffer(name);
  return playViaAudioElement(name);
}
