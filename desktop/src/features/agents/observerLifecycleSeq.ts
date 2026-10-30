import { normalizePubkey } from "@/shared/lib/pubkey";
import type { ObserverEvent } from "./ui/agentSessionTypes";

const appliedLifecycleSeqByPairNonce = new Map<string, number>();

function lifecycleSequenceKey(
  agentPubkey: string,
  payload: unknown,
): string | null {
  if (payload === null || typeof payload !== "object") return null;
  const record = payload as { relayUrl?: unknown; startNonce?: unknown };
  if (
    typeof record.relayUrl !== "string" ||
    typeof record.startNonce !== "string"
  ) {
    return null;
  }
  return `${normalizePubkey(agentPubkey)}\0${record.relayUrl}\0${record.startNonce}`;
}

export function shouldApplyLifecycleFrame(
  agentPubkey: string,
  event: ObserverEvent,
): boolean {
  const key = lifecycleSequenceKey(agentPubkey, event.payload);
  if (key === null) return false;
  const applied = appliedLifecycleSeqByPairNonce.get(key);
  return applied === undefined || event.seq > applied;
}

export function recordAppliedLifecycleFrame(
  agentPubkey: string,
  event: ObserverEvent,
): void {
  const key = lifecycleSequenceKey(agentPubkey, event.payload);
  if (key === null) return;
  appliedLifecycleSeqByPairNonce.set(key, event.seq);
}

export function resetAppliedLifecycleSeq(): void {
  appliedLifecycleSeqByPairNonce.clear();
}
