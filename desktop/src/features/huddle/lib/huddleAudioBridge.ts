import { invoke } from "@tauri-apps/api/core";

import type { AudioInputDevice } from "./useAudioDevices";
import type { VoiceInputMode } from "./useHuddlePttState";

/** Mirrored companion→main / main→companion microphone UI state. */
export type HuddleAudioMirrorState = {
  isMuted: boolean;
  micConnected: boolean;
  audioDevices: AudioInputDevice[];
  selectedDeviceId: string;
  micGain: number;
  voiceInputMode: VoiceInputMode;
};

export type HuddleAudioCommand =
  | { type: "request-state" }
  | { type: "set-muted"; isMuted: boolean }
  | { type: "set-input-device"; deviceId: string }
  | { type: "set-mic-gain"; gain: number }
  | { type: "set-voice-input-mode"; mode: VoiceInputMode };

export const HUDDLE_AUDIO_COMMAND_EVENT = "huddle-audio-command";
export const HUDDLE_AUDIO_STATE_EVENT = "huddle-audio-state";
export const HUDDLE_AUDIO_LEVEL_EVENT = "huddle-audio-level";

export function isRedundantHuddlePhaseError(message: string): boolean {
  return /^cannot (?:start|join) huddle: already in phase /i.test(message);
}

/** Interrupt this agent only if it still owns the active utterance. */
export function interruptAgentSpeech(agentPubkey: string) {
  return invoke<void>("interrupt_huddle_speech", { agentPubkey });
}
