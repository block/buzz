import type { AudioInputDevice } from "./lib/useAudioDevices";
import type { VoiceInputMode } from "./lib/useHuddlePttState";

/**
 * High-frequency audio-level fields, split from {@link HuddleContextValue} so
 * their 20-30 Hz updates only re-render the meter components that consume
 * them — not every `useHuddle()` consumer across the app.
 */
export interface HuddleLevelsValue {
  micLevel: number;
  activeSpeakers: string[];
  speakerLevels: Record<string, number>;
}

export interface HuddleContextValue {
  localAudioTrack: MediaStreamTrack | null;
  isStarting: boolean;
  huddleError: string | null;
  clearHuddleError: () => void;
  micConnected: boolean;
  isMuted: boolean;
  toggleMute: () => void;
  /** Interrupt this agent only if it still owns the active utterance. */
  interruptAgentSpeech: (agentPubkey: string) => Promise<void>;
  pttActive: boolean;
  voiceInputMode: VoiceInputMode;
  setVoiceInputMode: (mode: VoiceInputMode) => Promise<void>;
  audioDevices: AudioInputDevice[];
  selectedDeviceId: string;
  setSelectedDeviceId: (id: string) => void;
  micGain: number;
  setMicGain: (value: number) => void;
  outputDevices: { name: string; is_default: boolean }[];
  selectedOutputDevice: string;
  setSelectedOutputDevice: (name: string) => void;
  activeEphemeralChannelId: string | null;
  showHuddleInMainApp: (ephemeralChannelId: string) => void;
  viewHuddleChannel: (ephemeralChannelId: string) => void;
  startHuddle: (
    parentChannelId: string,
    memberPubkeys: string[],
    channelName?: string,
  ) => Promise<void>;
  joinHuddle: (
    parentChannelId: string,
    ephemeralChannelId: string,
    huddleThreadEventId?: string,
  ) => Promise<void>;
  leaveHuddle: () => Promise<boolean>;
}
