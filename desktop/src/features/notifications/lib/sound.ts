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
  "all_messages",
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
  all_messages: "All messages",
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
  all_messages: "When someone posts in a channel.",
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
  all_messages: "doop",
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
  all_messages: "flutter",
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
  all_messages: true,
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

const cache = new Map<SoundName, HTMLAudioElement>();
const loading = new Map<SoundName, Promise<HTMLAudioElement>>();
const decodedBuffers = new Map<SoundName, AudioBuffer>();
const decoding = new Map<SoundName, Promise<AudioBuffer | null>>();

let notificationAudioContext: AudioContext | null = null;
let unlockInstalled = false;

function soundUrl(name: SoundName): string {
  return `/sounds/${name}.mp3`;
}

function getNotificationAudioContext(): AudioContext | null {
  const ContextCtor =
    globalThis.AudioContext ??
    (
      globalThis as typeof globalThis & {
        webkitAudioContext?: typeof AudioContext;
      }
    ).webkitAudioContext;
  if (!ContextCtor) {
    return null;
  }
  try {
    notificationAudioContext ??= new ContextCtor({
      latencyHint: "interactive",
    });
    return notificationAudioContext;
  } catch {
    return null;
  }
}

async function fetchSoundBytes(name: SoundName): Promise<ArrayBuffer> {
  const response = await fetch(soundUrl(name));
  if (!response.ok) {
    throw new Error(`Failed to fetch notification sound (${response.status})`);
  }
  return response.arrayBuffer();
}

async function loadDecodedBuffer(name: SoundName): Promise<AudioBuffer | null> {
  const cached = decodedBuffers.get(name);
  if (cached) {
    return cached;
  }
  const inFlight = decoding.get(name);
  if (inFlight) {
    return inFlight;
  }

  const ctx = getNotificationAudioContext();
  if (!ctx) {
    return null;
  }

  const promise = (async () => {
    const bytes = await fetchSoundBytes(name);
    const audioBuffer = await ctx.decodeAudioData(bytes.slice(0));
    decodedBuffers.set(name, audioBuffer);
    return audioBuffer;
  })().catch((error) => {
    console.warn("[notifications] sound decode failed", name, error);
    return null;
  });

  decoding.set(name, promise);
  try {
    return await promise;
  } finally {
    decoding.delete(name);
  }
}

function playDecodedBuffer(buffer: AudioBuffer): boolean {
  const ctx = getNotificationAudioContext();
  if (!ctx) {
    return false;
  }
  try {
    const source = ctx.createBufferSource();
    source.buffer = buffer;
    source.connect(ctx.destination);
    source.start();
    return true;
  } catch (error) {
    console.warn("[notifications] AudioContext playback failed", error);
    return false;
  }
}

/**
 * WebKitGTK autoplay blocks HTMLAudio without a user gesture. Resume the
 * shared AudioContext (and decode the mp3s) on the first click/key so later
 * live alerts can play without a gesture.
 */
export function unlockNotificationAudio() {
  const ctx = getNotificationAudioContext();
  if (ctx?.state === "suspended") {
    void ctx.resume().catch(() => {});
  }
  for (const name of SOUND_NAMES) {
    void loadDecodedBuffer(name);
  }
}

function installNotificationAudioUnlock() {
  if (unlockInstalled || typeof window === "undefined") {
    return;
  }
  unlockInstalled = true;
  const unlock = () => {
    unlockNotificationAudio();
    window.removeEventListener("pointerdown", unlock);
    window.removeEventListener("keydown", unlock);
  };
  window.addEventListener("pointerdown", unlock);
  window.addEventListener("keydown", unlock);
}

installNotificationAudioUnlock();

/**
 * WebKitGTK's media backend cannot decode media from Tauri's custom URI
 * scheme (`tauri://…` / `http://tauri.localhost`), so `new Audio("/sounds/…")`
 * fails with `MEDIA_ERR_SRC_NOT_SUPPORTED`. Fetching the asset through JS and
 * handing the element a `blob:` URL works because `media-src` allows `blob:`
 * (block/buzz#2562).
 */
async function loadAudio(name: SoundName): Promise<HTMLAudioElement> {
  const cached = cache.get(name);
  if (cached) {
    return cached;
  }

  const inFlight = loading.get(name);
  if (inFlight) {
    return inFlight;
  }

  const promise = (async () => {
    const buffer = await fetchSoundBytes(name);
    const blob = new Blob([buffer], { type: "audio/mpeg" });
    const audio = new Audio(URL.createObjectURL(blob));
    cache.set(name, audio);
    return audio;
  })();

  loading.set(name, promise);
  try {
    return await promise;
  } finally {
    loading.delete(name);
  }
}

/** @internal Test-only: drop cached elements so load paths can be re-exercised. */
export function resetNotificationSoundCache() {
  cache.clear();
  loading.clear();
  decodedBuffers.clear();
  decoding.clear();
  notificationAudioContext = null;
}

export async function playNotificationSound(
  name: SoundName,
  options?: { preview?: boolean },
): Promise<HTMLAudioElement | null> {
  const preview = options?.preview === true;
  try {
    // Settings preview is a click, so HTMLAudio.play() is allowed and gives
    // pause/ended. Live alerts have no gesture: WebKitGTK often *resolves*
    // play() while staying silent, so AudioContext must go first.
    if (!preview) {
      const ctx = getNotificationAudioContext();
      if (ctx?.state === "suspended") {
        await ctx.resume().catch(() => {});
      }
      if (ctx?.state === "running") {
        const decoded = await loadDecodedBuffer(name);
        if (decoded && playDecodedBuffer(decoded)) {
          return cache.get(name) ?? (await loadAudio(name).catch(() => null));
        }
      }
    }

    const audio = await loadAudio(name);
    audio.currentTime = 0;
    await audio.play();
    return audio;
  } catch (error) {
    console.warn("[notifications] sound play failed", name, error);
    return null;
  }
}
