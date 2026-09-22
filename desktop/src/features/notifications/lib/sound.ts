import { i18n } from "@/i18n";
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

/**
 * Row labels for the alert-sound slots in the notification settings.
 *
 * Resolved through getters rather than stored as module-level literals:
 * `i18n` boots after this module is imported, while the settings card keeps
 * reading plain `SLOT_LABELS[slot]` at render time.
 */
export const SLOT_LABELS: Record<SoundSlot, string> = {
  get dm() {
    return i18n.t("sidebar.shell.direct-messages");
  },
  get mention() {
    return i18n.t("notifications.sound.slot-mentions");
  },
  get thread_reply() {
    return i18n.t("notifications.sound.slot-thread-replies");
  },
  get needs_action() {
    return i18n.t("notifications.sound.slot-needs-action");
  },
  get job_accepted() {
    return i18n.t("notifications.sound.slot-agent-job-accepted");
  },
  get job_progress() {
    return i18n.t("notifications.sound.slot-agent-job-progress");
  },
  get job_result() {
    return i18n.t("notifications.sound.slot-agent-job-result");
  },
  get job_error() {
    return i18n.t("notifications.sound.slot-agent-job-error");
  },
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

/**
 * Slot explanations shown as the settings row subcopy (same lazy resolution as
 * {@link SLOT_LABELS}).
 */
export const SLOT_DESCRIPTIONS: Record<SoundSlot, string> = {
  get dm() {
    return i18n.t("notifications.sound.desc-direct-messages");
  },
  get mention() {
    return i18n.t("notifications.sound.desc-mentions");
  },
  get thread_reply() {
    return i18n.t("notifications.sound.desc-thread-replies");
  },
  get needs_action() {
    return i18n.t("notifications.sound.desc-needs-action");
  },
  get job_accepted() {
    return i18n.t("notifications.sound.desc-agent-job-accepted");
  },
  get job_progress() {
    return i18n.t("notifications.sound.desc-agent-job-progress");
  },
  get job_result() {
    return i18n.t("notifications.sound.desc-agent-job-result");
  },
  get job_error() {
    return i18n.t("notifications.sound.desc-agent-job-error");
  },
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

const cache = new Map<SoundName, HTMLAudioElement>();

function getAudio(name: SoundName): HTMLAudioElement {
  let audio = cache.get(name);
  if (!audio) {
    audio = new Audio(`/sounds/${name}.mp3`);
    cache.set(name, audio);
  }
  return audio;
}

export function playNotificationSound(
  name: SoundName,
): HTMLAudioElement | null {
  try {
    const audio = getAudio(name);
    audio.currentTime = 0;
    audio.play().catch(() => {
      // Best-effort — user may not have interacted with the page yet.
    });
    return audio;
  } catch {
    // Best-effort only.
    return null;
  }
}
