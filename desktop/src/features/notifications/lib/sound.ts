import {
  KIND_APPROVAL_REQUEST,
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

export function slotForFeedKind(
  kind: number,
  category: FeedItemCategory,
): SoundSlot {
  if (category === "mention") return "mention";
  if (kind === KIND_JOB_ACCEPTED) return "job_accepted";
  if (kind === KIND_JOB_PROGRESS) return "job_progress";
  if (kind === KIND_JOB_RESULT) return "job_result";
  if (kind === KIND_JOB_ERROR) return "job_error";
  if (kind === KIND_APPROVAL_REQUEST) return "needs_action";
  return "needs_action";
}

export function shouldPlayNotificationSound(
  channelId: string | null | undefined,
  silentChannelIds?: ReadonlySet<string>,
): boolean {
  return !channelId || !silentChannelIds?.has(channelId);
}

const bufferCache = new Map<SoundName, Promise<AudioBuffer>>();
let audioContext: AudioContext | null = null;
const activePlaybacks = new Map<SoundName, SoundPlayback>();

export type SoundPlayback = {
  stop: () => void;
  onEnded: (listener: () => void) => () => void;
};

function getAudioContext(): AudioContext {
  audioContext ??= new AudioContext({ latencyHint: "interactive" });
  return audioContext;
}

function getAudioBuffer(
  context: AudioContext,
  name: SoundName,
): Promise<AudioBuffer> {
  const cached = bufferCache.get(name);
  if (cached) return cached;

  const pending = fetch(`/sounds/${name}.mp3`)
    .then((response) => {
      if (!response.ok) {
        throw new Error(
          `Failed to load notification sound: ${response.status}`,
        );
      }
      return response.arrayBuffer();
    })
    .then((data) => context.decodeAudioData(data))
    .catch((error) => {
      if (bufferCache.get(name) === pending) {
        bufferCache.delete(name);
      }
      throw error;
    });
  bufferCache.set(name, pending);
  return pending;
}

function createPlayback(): {
  playback: SoundPlayback;
  setSource: (source: AudioBufferSourceNode) => void;
  finish: () => void;
  isStopped: () => boolean;
} {
  let source: AudioBufferSourceNode | null = null;
  let stopped = false;
  let ended = false;
  const listeners = new Set<() => void>();

  const finish = () => {
    if (ended) return;
    ended = true;
    for (const listener of listeners) listener();
    listeners.clear();
  };

  return {
    playback: {
      stop: () => {
        if (stopped) return;
        stopped = true;
        try {
          source?.stop();
        } catch {
          // The source may not have started yet.
        }
        finish();
      },
      onEnded: (listener) => {
        if (ended) {
          queueMicrotask(listener);
          return () => {};
        }
        listeners.add(listener);
        return () => listeners.delete(listener);
      },
    },
    setSource: (nextSource) => {
      source = nextSource;
    },
    finish,
    isStopped: () => stopped,
  };
}

export function playNotificationSound(name: SoundName): SoundPlayback | null {
  try {
    const context = getAudioContext();
    activePlaybacks.get(name)?.stop();

    const controller = createPlayback();
    activePlaybacks.set(name, controller.playback);
    controller.playback.onEnded(() => {
      if (activePlaybacks.get(name) === controller.playback) {
        activePlaybacks.delete(name);
      }
    });

    void (async () => {
      try {
        const buffer = await getAudioBuffer(context, name);
        if (controller.isStopped()) return;
        if (context.state === "suspended") await context.resume();
        if (controller.isStopped()) return;

        const source = context.createBufferSource();
        source.buffer = buffer;
        source.connect(context.destination);
        source.addEventListener("ended", controller.finish, { once: true });
        controller.setSource(source);
        source.start();
      } catch {
        // Best-effort — audio can be blocked or unavailable.
        controller.finish();
      }
    })();

    return controller.playback;
  } catch {
    // Best-effort only.
    return null;
  }
}
