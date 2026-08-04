import type { AudioInputDevice } from "./lib/useAudioDevices";
import type { VoiceInputMode } from "./lib/useHuddlePttState";

export interface HuddleContextValue {
  localAudioTrack: MediaStreamTrack | null;
  isStarting: boolean;
  huddleError: string | null;
  clearHuddleError: () => void;
  micConnected: boolean;
  isMuted: boolean;
  toggleMute: () => void;
  /** Interrupt current agent speech and discard queued TTS responses. */
  interruptAgentSpeech: () => Promise<void>;
  micLevel: number;
  pttActive: boolean;
  voiceInputMode: VoiceInputMode;
  setVoiceInputMode: (mode: VoiceInputMode) => Promise<void>;
  activeSpeakers: string[];
  speakerLevels: Record<string, number>;
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
