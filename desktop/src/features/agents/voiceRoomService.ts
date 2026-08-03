import {
  endVoiceTarget,
  saveVoiceTargetPreference,
  setVoiceRoomOutputMuted,
  setVoiceTargetMuted,
  setVoiceTargetVoice,
  startVoiceTarget,
  type CodexVoiceTarget,
  type CodexVoiceTargetInput,
  VOICE_ROOM_PALETTE,
} from "@/features/agents/voiceSessionRegistry";

export type VoiceRoomAgentRef = {
  agentName?: string;
  agentPubkey?: string;
  threadId?: string;
};

export type VoiceRoomCommand =
  | ({ action: "join" } & VoiceRoomAgentRef)
  | ({ action: "remove" } & VoiceRoomAgentRef)
  | ({ action: "set-muted"; muted: boolean } & VoiceRoomAgentRef)
  | ({ action: "set-voice"; voice: string } & VoiceRoomAgentRef)
  | { action: "set-output-muted"; muted: boolean };

export type VoiceRoomCommandResult =
  | { ok: true; action: VoiceRoomCommand["action"]; threadId?: string }
  | { ok: false; action: VoiceRoomCommand["action"]; error: string };

export const VOICE_ROOM_COMMAND_EVENT = "buzz:voice-room-command";
export const VOICE_ROOM_COMMAND_RESULT_EVENT = "buzz:voice-room-command-result";
export const VOICE_ROOM_COMMAND_REQUEST = "voice_room_command";

export type VoiceRoomCommandRequest = {
  type: typeof VOICE_ROOM_COMMAND_REQUEST;
  command: VoiceRoomCommand;
  requestId: string;
};

export function parseVoiceRoomCommandRequest(
  value: unknown,
): VoiceRoomCommandRequest | null {
  if (!value || typeof value !== "object") return null;
  const request = value as Record<string, unknown>;
  if (
    request.type !== VOICE_ROOM_COMMAND_REQUEST ||
    typeof request.requestId !== "string" ||
    !request.requestId.trim() ||
    !request.command ||
    typeof request.command !== "object"
  ) {
    return null;
  }
  const command = request.command as Record<string, unknown>;
  const action = command.action;
  if (
    action !== "join" &&
    action !== "remove" &&
    action !== "set-muted" &&
    action !== "set-voice" &&
    action !== "set-output-muted"
  ) {
    return null;
  }
  const allowed =
    action === "set-output-muted"
      ? ["action", "muted"]
      : action === "set-muted"
        ? ["action", "agentName", "agentPubkey", "threadId", "muted"]
        : action === "set-voice"
          ? ["action", "agentName", "agentPubkey", "threadId", "voice"]
          : ["action", "agentName", "agentPubkey", "threadId"];
  if (Object.keys(command).some((key) => !allowed.includes(key))) return null;
  const hasAgentRef = [
    command.agentName,
    command.agentPubkey,
    command.threadId,
  ].some((candidate) => typeof candidate === "string" && candidate.trim());
  if (action !== "set-output-muted" && !hasAgentRef) return null;
  if (
    (action === "set-muted" || action === "set-output-muted") &&
    typeof command.muted !== "boolean"
  ) {
    return null;
  }
  if (
    action === "set-voice" &&
    (typeof command.voice !== "string" || !command.voice.trim())
  ) {
    return null;
  }
  return request as unknown as VoiceRoomCommandRequest;
}

let availableTargets: CodexVoiceTargetInput[] = [];
let activeTargets: readonly CodexVoiceTarget[] = [];

export function updateVoiceRoomCommandContext(input: {
  activeTargets: readonly CodexVoiceTarget[];
  availableTargets: readonly CodexVoiceTargetInput[];
}) {
  activeTargets = input.activeTargets;
  availableTargets = [...input.availableTargets];
}

export function executeVoiceRoomCommand(
  command: VoiceRoomCommand,
): VoiceRoomCommandResult {
  if (command.action === "set-output-muted") {
    setVoiceRoomOutputMuted(command.muted);
    return { ok: true, action: command.action };
  }

  const active = findAgent(activeTargets, command);
  const available = findAgent(availableTargets, command);

  if (command.action === "join") {
    if (active)
      return { ok: true, action: command.action, threadId: active.threadId };
    if (!available)
      return failure(command, "Agent is not available for voice.");
    startVoiceTarget(available);
    return { ok: true, action: command.action, threadId: available.threadId };
  }

  if (!active)
    return failure(command, "Agent is not active in the voice room.");

  if (command.action === "remove") {
    endVoiceTarget(active.threadId);
  } else if (command.action === "set-muted") {
    setVoiceTargetMuted(active.threadId, command.muted);
  } else if (command.action === "set-voice") {
    if (
      !VOICE_ROOM_PALETTE.includes(
        command.voice as (typeof VOICE_ROOM_PALETTE)[number],
      )
    ) {
      return failure(command, "Voice is not supported.");
    }
    setVoiceTargetVoice(active.threadId, command.voice);
    saveVoiceTargetPreference({ ...active, voice: command.voice });
  }
  return { ok: true, action: command.action, threadId: active.threadId };
}

export function installVoiceRoomCommandBridge() {
  if (typeof window === "undefined") return () => undefined;
  const handleCommand = (event: Event) => {
    const request = (event as CustomEvent<VoiceRoomCommandRequest>).detail;
    const parsed = parseVoiceRoomCommandRequest(request);
    if (!parsed) return;
    const result = executeVoiceRoomCommand(parsed.command);
    window.dispatchEvent(
      new CustomEvent(VOICE_ROOM_COMMAND_RESULT_EVENT, {
        detail: { requestId: parsed.requestId, result },
      }),
    );
  };
  window.addEventListener(VOICE_ROOM_COMMAND_EVENT, handleCommand);
  return () =>
    window.removeEventListener(VOICE_ROOM_COMMAND_EVENT, handleCommand);
}

function findAgent<T extends CodexVoiceTargetInput>(
  targets: readonly T[],
  reference: VoiceRoomAgentRef,
): T | undefined {
  const name = reference.agentName?.trim().toLowerCase();
  const pubkey = reference.agentPubkey?.trim().toLowerCase();
  return targets.find(
    (target) =>
      (reference.threadId && target.threadId === reference.threadId) ||
      (pubkey && target.agentPubkey.toLowerCase() === pubkey) ||
      (name && target.agentName.trim().toLowerCase() === name),
  );
}

function failure(
  command: VoiceRoomCommand,
  error: string,
): VoiceRoomCommandResult {
  return { ok: false, action: command.action, error };
}
