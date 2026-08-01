import {
  KIND_APPROVAL_REQUEST,
  KIND_JOB_ACCEPTED,
  KIND_JOB_ERROR,
  KIND_JOB_PROGRESS,
  KIND_JOB_RESULT,
} from "@/shared/constants/kinds";
import type { FeedItemCategory } from "@/shared/api/types";

export const SOUND_NAMES = [
  "amp",
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

/**
 * Message-like slots resolve their sound by who sent the message (human vs
 * agent) rather than by the per-slot picks. Non-message slots (needs_action,
 * job_*) keep per-slot sounds.
 */
export const SENDER_SOUND_SLOTS: ReadonlySet<SoundSlot> = new Set([
  "dm",
  "mention",
  "thread_reply",
]);

export const SENDER_KINDS = ["human", "agent"] as const;
export type SenderKind = (typeof SENDER_KINDS)[number];

export type SenderSounds = Record<SenderKind, SoundName>;

export const SENDER_LABELS: Record<SenderKind, string> = {
  human: "Human senders",
  agent: "Agent senders",
};

export const SENDER_DESCRIPTIONS: Record<SenderKind, string> = {
  human: "Messages, mentions, and replies sent by people.",
  agent: "Messages, mentions, and replies sent by agents.",
};

export const DEFAULT_SENDER_SOUNDS: SenderSounds = {
  human: "ping",
  agent: "amp",
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
  senderSounds: SenderSounds;
};

export function resolveSlotSound(
  prefs: SoundPreferences,
  slot: SoundSlot,
): SoundName {
  return prefs.sounds[slot];
}

/** Audible message policy: real mentions use Amp; unread-only slots are silent. */
export function notificationSoundForSlot(slot: SoundSlot): SoundName | null {
  return slot === "mention" ? "amp" : null;
}

/** Automatic sound policy. Explicit agent sound tags are handled separately. */
export function resolveEventSound(
  prefs: SoundPreferences,
  slot: SoundSlot,
  isAgentSender: boolean,
): SoundName | null {
  if (SENDER_SOUND_SLOTS.has(slot)) {
    // Agent-authored messages are always silent unless their event carries an
    // explicit sound request, which is evaluated by the live message path.
    if (isAgentSender) return null;
    return notificationSoundForSlot(slot);
  }
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
