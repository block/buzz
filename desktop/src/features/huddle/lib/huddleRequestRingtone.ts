import type { SoundName } from "@/features/notifications/lib/sound";
import {
  KIND_HUDDLE_ENDED,
  KIND_HUDDLE_STARTED,
} from "@/shared/constants/kinds";

export const HUDDLE_REQUEST_RING_TIMEOUT_MS = 30_000;

type RingtoneAudio = Pick<
  HTMLAudioElement,
  "currentTime" | "loop" | "pause" | "play"
>;

type RingtoneDependencies = {
  createAudio: (source: string) => RingtoneAudio;
  scheduleTimeout: (callback: () => void, delayMs: number) => number;
  clearScheduledTimeout: (timeoutId: number) => void;
};

export type HuddleRequestRingtoneController = {
  start: (huddleId: string, sound: SoundName) => boolean;
  stop: (huddleId?: string) => boolean;
};

const browserDependencies: RingtoneDependencies = {
  createAudio: (source) => new Audio(source),
  scheduleTimeout: (callback, delayMs) => window.setTimeout(callback, delayMs),
  clearScheduledTimeout: (timeoutId) => window.clearTimeout(timeoutId),
};

export function createHuddleRequestRingtoneController(
  dependencies: RingtoneDependencies = browserDependencies,
): HuddleRequestRingtoneController {
  let active:
    | { audio: RingtoneAudio; huddleId: string; timeoutId: number }
    | undefined;

  function stop(huddleId?: string) {
    if (!active || (huddleId && active.huddleId !== huddleId)) {
      return false;
    }

    const current = active;
    active = undefined;
    dependencies.clearScheduledTimeout(current.timeoutId);
    current.audio.pause();
    current.audio.currentTime = 0;
    return true;
  }

  function start(huddleId: string, sound: SoundName) {
    if (active?.huddleId === huddleId) {
      return false;
    }

    stop();
    const audio = dependencies.createAudio(`/sounds/${sound}.mp3`);
    audio.loop = true;
    audio.currentTime = 0;
    const timeoutId = dependencies.scheduleTimeout(
      () => stop(huddleId),
      HUDDLE_REQUEST_RING_TIMEOUT_MS,
    );
    active = { audio, huddleId, timeoutId };
    Promise.resolve(audio.play()).catch(() => {
      // Best-effort — browser autoplay policy may block audio until interaction.
    });
    return true;
  }

  return { start, stop };
}

export function shouldRingForHuddleRequest({
  currentPubkey,
  enabled,
  initiatorPubkey,
  muted,
}: {
  currentPubkey?: string;
  enabled: boolean;
  initiatorPubkey: string;
  muted: boolean;
}) {
  const normalizedCurrentPubkey = currentPubkey?.trim().toLowerCase();
  return (
    enabled &&
    !muted &&
    Boolean(normalizedCurrentPubkey) &&
    normalizedCurrentPubkey !== initiatorPubkey.toLowerCase()
  );
}

export function huddleIdFromLifecycleContent(content: string): string | null {
  try {
    const parsed = JSON.parse(content) as { ephemeral_channel_id?: unknown };
    return typeof parsed.ephemeral_channel_id === "string" &&
      parsed.ephemeral_channel_id.length > 0
      ? parsed.ephemeral_channel_id
      : null;
  } catch {
    return null;
  }
}

export function huddleRingtoneCommand(
  kind: number,
  content: string,
): { action: "start" | "stop"; huddleId: string } | null {
  const huddleId = huddleIdFromLifecycleContent(content);
  if (!huddleId) return null;
  if (kind === KIND_HUDDLE_STARTED) return { action: "start", huddleId };
  if (kind === KIND_HUDDLE_ENDED) return { action: "stop", huddleId };
  return null;
}

export const huddleRequestRingtone = createHuddleRequestRingtoneController();
