export type AgentVoiceReadiness =
  | { action: "enable_transcription"; message: string }
  | { action: "unmute"; message: string }
  | { action: null; message: string }
  | null;

type AgentVoiceReadinessInput = {
  hasAgents: boolean;
  isMuted: boolean;
  isPttMode: boolean;
  micConnected: boolean;
  pushToTalkShortcut: string;
  transcriptionEnabled: boolean;
};

/**
 * Returns the blocking reason an enrolled agent cannot receive spoken input.
 * The UI keeps microphone activation explicit while making the failure state
 * visible instead of silently producing an empty transcript.
 */
export function getAgentVoiceReadiness({
  hasAgents,
  isMuted,
  isPttMode,
  micConnected,
  pushToTalkShortcut,
  transcriptionEnabled,
}: AgentVoiceReadinessInput): AgentVoiceReadiness {
  if (!hasAgents) return null;
  if (!micConnected) {
    return {
      action: null,
      message: "Microphone unavailable — agents cannot hear you.",
    };
  }
  if (!transcriptionEnabled) {
    return {
      action: "enable_transcription",
      message: "Transcript is off — turn it on so agents can hear you.",
    };
  }
  if (!isMuted) return null;
  return {
    action: "unmute",
    message: isPttMode
      ? `Mic muted — click to unmute or hold ${pushToTalkShortcut} to talk to agents.`
      : "Mic muted — click to unmute and talk to agents.",
  };
}
