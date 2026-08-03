import { invokeTauri } from "@/shared/api/tauri";

export type CodexVoiceCapability = {
  supported: boolean;
  reason: string | null;
  model: string | null;
  mode: CodexVoiceMode | null;
};

export type CodexVoiceMode = "native" | "proxy";

export type CodexVoiceTargetLink = {
  channelId: string;
  threadId: string;
};

export type CodexVoiceStartResponse = {
  muted: boolean;
  model: string;
  mode: CodexVoiceMode;
  voice: string;
};

export type CodexVoiceStatus = {
  active: boolean;
  muted: boolean;
  model: string | null;
  sessions: Array<{
    threadId: string;
    muted: boolean;
    model: string;
    mode: CodexVoiceMode;
    voice: string;
  }>;
};

export type CodexVoiceEvent = {
  method: string;
  params: {
    threadId?: string;
    sdp?: string;
    role?: string;
    delta?: string;
    text?: string;
    message?: string;
    reason?: string | null;
  };
};

let voiceLinkRevision = 0;
const voiceLinkListeners = new Set<() => void>();

export function subscribeCodexVoiceLinkChanges(listener: () => void) {
  voiceLinkListeners.add(listener);
  return () => {
    voiceLinkListeners.delete(listener);
  };
}

export function getCodexVoiceLinkRevision() {
  return voiceLinkRevision;
}

function notifyCodexVoiceLinkChange() {
  voiceLinkRevision += 1;
  for (const listener of voiceLinkListeners) listener();
}

export function getCodexVoiceCapability(
  pubkey: string,
  relayUrl: string,
): Promise<CodexVoiceCapability> {
  return invokeTauri("get_codex_voice_capability", { pubkey, relayUrl });
}

export function requestMicrophoneAccess(): Promise<boolean> {
  return invokeTauri("request_microphone_access");
}

export function getCodexVoiceStatus(): Promise<CodexVoiceStatus> {
  return invokeTauri("get_codex_voice_status");
}

export function getCodexVoiceLink(
  pubkey: string,
  channelId: string,
): Promise<string | null> {
  return invokeTauri("get_codex_voice_link", { pubkey, channelId });
}

export function getCodexVoiceTargetLink(
  pubkey: string,
): Promise<CodexVoiceTargetLink | null> {
  return invokeTauri("get_codex_voice_target_link", { pubkey });
}

export async function rememberCodexVoiceLink(
  pubkey: string,
  channelId: string,
  threadId: string,
): Promise<void> {
  await invokeTauri("remember_codex_voice_link", {
    pubkey,
    channelId,
    threadId,
  });
  notifyCodexVoiceLinkChange();
}

export function startCodexVoice(input: {
  threadId: string;
  pubkey: string;
  agentName: string;
  relayUrl: string;
  voice: string;
  sdp: string;
}): Promise<CodexVoiceStartResponse> {
  return invokeTauri("start_codex_voice", input);
}

export function speakCodexVoice(threadId: string, text: string): Promise<void> {
  return invokeTauri("speak_codex_voice", { threadId, text });
}

export function stopCodexVoice(threadId: string): Promise<void> {
  return invokeTauri("stop_codex_voice", { threadId });
}

export function setCodexVoiceMuted(
  threadId: string,
  muted: boolean,
): Promise<boolean> {
  return invokeTauri("set_codex_voice_muted", { threadId, muted });
}
