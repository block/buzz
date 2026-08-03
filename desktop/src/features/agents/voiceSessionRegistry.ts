import * as React from "react";

import type { CodexVoiceMode } from "@/shared/api/codexVoice";
import { resolveVoiceTurnRecipient } from "@/features/agents/voiceTurnRouting";

export type CodexVoiceTarget = {
  agentName: string;
  agentPubkey: string;
  channelId: string;
  mode: CodexVoiceMode;
  muted?: boolean;
  relayUrl: string;
  threadId: string;
  voice: string;
};

export type CodexVoiceTargetInput = Omit<CodexVoiceTarget, "voice"> & {
  voice?: string;
};

export type CodexVoiceSessionState = {
  error: string | null;
  muted: boolean;
  phase: "starting" | "listening" | "ending" | "error";
  transcript: string | null;
};

export type VoiceRoomTranscriptEntry = {
  id: number;
  speakerName: string;
  speakerType: "agent" | "human";
  text: string;
  timestamp: number;
};

export type VoiceRoomDirectedTurn = {
  id: number;
  recipientThreadId: string;
  text: string;
};

export type VoiceRoomSpeakerLease = {
  threadId: string;
  turnId: number;
};

export const VOICE_ROOM_PALETTE = [
  "sol",
  "cove",
  "ember",
  "breeze",
  "arbor",
  "vale",
  "juniper",
  "maple",
  "spruce",
] as const;

const ACTIVE_STORAGE_KEY = "buzz.voice-room.active.v1";
const SAVED_STORAGE_KEY = "buzz.voice-room.saved.v1";

function readTargets(key: string): CodexVoiceTarget[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const parsed = JSON.parse(localStorage.getItem(key) ?? "[]");
    return Array.isArray(parsed) ? parsed.filter(isPersistedVoiceTarget) : [];
  } catch {
    return [];
  }
}

function isPersistedVoiceTarget(value: unknown): value is CodexVoiceTarget {
  if (!value || typeof value !== "object") return false;
  const target = value as Partial<CodexVoiceTarget>;
  return (
    typeof target.agentName === "string" &&
    typeof target.agentPubkey === "string" &&
    typeof target.channelId === "string" &&
    (target.mode === "native" || target.mode === "proxy") &&
    typeof target.relayUrl === "string" &&
    typeof target.threadId === "string" &&
    typeof target.voice === "string"
  );
}

function persistTargets(key: string, targets: readonly CodexVoiceTarget[]) {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(key, JSON.stringify(targets));
  } catch {
    // Voice remains usable when storage is unavailable.
  }
}

export function chooseAvailableVoice(
  targets: readonly CodexVoiceTarget[],
): string {
  const assigned = new Set(targets.map((target) => target.voice));
  return (
    VOICE_ROOM_PALETTE.find((voice) => !assigned.has(voice)) ??
    VOICE_ROOM_PALETTE[targets.length % VOICE_ROOM_PALETTE.length]
  );
}

export function addVoiceTarget(
  targets: readonly CodexVoiceTarget[],
  target: CodexVoiceTarget,
): CodexVoiceTarget[] {
  if (targets.some((current) => current.threadId === target.threadId)) {
    return [...targets];
  }
  return [...targets, target];
}

export function removeVoiceTarget(
  targets: readonly CodexVoiceTarget[],
  threadId: string,
): CodexVoiceTarget[] {
  return targets.filter((target) => target.threadId !== threadId);
}

export function hasVoiceTarget(
  targets: readonly CodexVoiceTarget[],
  threadId: string,
): boolean {
  return targets.some((target) => target.threadId === threadId);
}

let activeTargets: CodexVoiceTarget[] = readTargets(ACTIVE_STORAGE_KEY);
let savedTargets: CodexVoiceTarget[] = readTargets(SAVED_STORAGE_KEY);
let sessionStates: Record<string, CodexVoiceSessionState> = {};
let roomTranscript: VoiceRoomTranscriptEntry[] = [];
let roomTranscriptSequence = 0;
let roomOutputMuted = false;
let directedTurns: VoiceRoomDirectedTurn[] = [];
let directedTurnSequence = 0;
let speakerLease: VoiceRoomSpeakerLease | null = null;
let speakerLeaseTimer: ReturnType<typeof setTimeout> | null = null;
const listeners = new Set<() => void>();

function emitChange() {
  for (const listener of listeners) listener();
}

function replaceActiveTargets(next: CodexVoiceTarget[]) {
  activeTargets = next;
  persistTargets(ACTIVE_STORAGE_KEY, next);
  emitChange();
}

function rememberTarget(target: CodexVoiceTarget) {
  savedTargets = [
    target,
    ...savedTargets.filter(
      (saved) =>
        saved.agentPubkey.toLowerCase() !== target.agentPubkey.toLowerCase(),
    ),
  ];
  persistTargets(SAVED_STORAGE_KEY, savedTargets);
}

export function startVoiceTarget(target: CodexVoiceTargetInput) {
  const saved = savedTargets.find(
    (current) =>
      current.agentPubkey.toLowerCase() === target.agentPubkey.toLowerCase(),
  );
  const completeTarget: CodexVoiceTarget = {
    ...target,
    muted: target.muted ?? saved?.muted ?? false,
    voice: target.voice ?? saved?.voice ?? chooseAvailableVoice(activeTargets),
  };
  const next = addVoiceTarget(activeTargets, completeTarget);
  rememberTarget(completeTarget);
  if (next.length === activeTargets.length) return;
  replaceActiveTargets(next);
}

export function saveVoiceTargetPreference(target: CodexVoiceTargetInput) {
  const saved = savedTargets.find(
    (current) =>
      current.agentPubkey.toLowerCase() === target.agentPubkey.toLowerCase(),
  );
  const completeTarget: CodexVoiceTarget = {
    ...target,
    muted: target.muted ?? saved?.muted ?? false,
    voice: target.voice ?? saved?.voice ?? chooseAvailableVoice(activeTargets),
  };
  rememberTarget(completeTarget);
  emitChange();
}

export function endVoiceTarget(threadId: string) {
  const next = removeVoiceTarget(activeTargets, threadId);
  if (next.length === activeTargets.length) return;
  const { [threadId]: _removed, ...remainingStates } = sessionStates;
  sessionStates = remainingStates;
  replaceActiveTargets(next);
}

export function setVoiceTargetMuted(threadId: string, muted: boolean) {
  const next = activeTargets.map((target) =>
    target.threadId === threadId ? { ...target, muted } : target,
  );
  if (next.every((target, index) => target === activeTargets[index])) return;
  const changed = next.find((target) => target.threadId === threadId);
  if (changed) rememberTarget(changed);
  replaceActiveTargets(next);
}

export function setVoiceTargetVoice(threadId: string, voice: string) {
  if (
    !VOICE_ROOM_PALETTE.includes(voice as (typeof VOICE_ROOM_PALETTE)[number])
  ) {
    return;
  }
  const next = activeTargets.map((target) =>
    target.threadId === threadId ? { ...target, voice } : target,
  );
  if (next.every((target, index) => target === activeTargets[index])) return;
  const changed = next.find((target) => target.threadId === threadId);
  if (changed) rememberTarget(changed);
  replaceActiveTargets(next);
}

export function updateVoiceSessionState(
  threadId: string,
  update: Partial<CodexVoiceSessionState>,
) {
  const current = sessionStates[threadId] ?? {
    error: null,
    muted: false,
    phase: "starting",
    transcript: null,
  };
  sessionStates = {
    ...sessionStates,
    [threadId]: { ...current, ...update },
  };
  emitChange();
}

export function appendVoiceRoomTranscript(input: {
  speakerName: string;
  speakerType: "agent" | "human";
  text: string;
}) {
  const text = input.text.trim();
  if (!text) return;
  const timestamp = Date.now();
  const duplicateWindowMs = input.speakerType === "human" ? 3_000 : 1_000;
  const duplicate = [...roomTranscript]
    .reverse()
    .find(
      (entry) =>
        timestamp - entry.timestamp <= duplicateWindowMs &&
        entry.speakerType === input.speakerType &&
        entry.speakerName === input.speakerName &&
        entry.text === text,
    );
  if (duplicate) return;
  roomTranscriptSequence += 1;
  roomTranscript = [
    ...roomTranscript.slice(-199),
    { ...input, id: roomTranscriptSequence, text, timestamp },
  ];
  emitChange();
}

export function setVoiceRoomOutputMuted(muted: boolean) {
  if (roomOutputMuted === muted) return;
  roomOutputMuted = muted;
  emitChange();
}

export function routeVoiceRoomTurn(text: string): VoiceRoomDirectedTurn | null {
  const recipient = resolveVoiceTurnRecipient(text, activeTargets);
  if (!recipient) return null;
  directedTurnSequence += 1;
  const turn = {
    id: directedTurnSequence,
    recipientThreadId: recipient.threadId,
    text: text.trim(),
  };
  directedTurns = [...directedTurns.slice(-49), turn];
  claimVoiceRoomSpeaker(recipient.threadId, turn.id);
  emitChange();
  return turn;
}

export function claimVoiceRoomSpeaker(threadId: string, turnId: number) {
  if (speakerLeaseTimer) clearTimeout(speakerLeaseTimer);
  speakerLease = { threadId, turnId };
  speakerLeaseTimer = setTimeout(() => {
    if (speakerLease?.turnId !== turnId) return;
    speakerLease = null;
    speakerLeaseTimer = null;
    emitChange();
  }, 45_000);
  emitChange();
}

export function releaseVoiceRoomSpeaker(threadId: string) {
  if (speakerLease?.threadId !== threadId) return;
  if (speakerLeaseTimer) clearTimeout(speakerLeaseTimer);
  speakerLeaseTimer = null;
  speakerLease = null;
  emitChange();
}

export function getVoiceRoomSnapshot() {
  return {
    activeTargets,
    outputMuted: roomOutputMuted,
    directedTurns,
    speakerLease,
  } as const;
}

export function useVoiceRoomDirectedTurns(): readonly VoiceRoomDirectedTurn[] {
  return React.useSyncExternalStore(
    subscribe,
    () => directedTurns,
    () => directedTurns,
  );
}

export function useVoiceRoomSpeakerLease(): VoiceRoomSpeakerLease | null {
  return React.useSyncExternalStore(
    subscribe,
    () => speakerLease,
    () => speakerLease,
  );
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useCodexVoiceTargets(): readonly CodexVoiceTarget[] {
  return React.useSyncExternalStore(
    subscribe,
    () => activeTargets,
    () => activeTargets,
  );
}

export function useSavedCodexVoiceTargets(): readonly CodexVoiceTarget[] {
  return React.useSyncExternalStore(
    subscribe,
    () => savedTargets,
    () => savedTargets,
  );
}

export function useCodexVoiceSessionStates(): Readonly<
  Record<string, CodexVoiceSessionState>
> {
  return React.useSyncExternalStore(
    subscribe,
    () => sessionStates,
    () => sessionStates,
  );
}

export function useVoiceRoomTranscript(): readonly VoiceRoomTranscriptEntry[] {
  return React.useSyncExternalStore(
    subscribe,
    () => roomTranscript,
    () => roomTranscript,
  );
}

export function useVoiceRoomOutputMuted(): boolean {
  return React.useSyncExternalStore(
    subscribe,
    () => roomOutputMuted,
    () => roomOutputMuted,
  );
}
