// Push diagnostics shown in Settings → Notifications (web) and logged to the ?kbdebug=1 overlay. Never holds secrets:
// only states, timestamps, HTTP statuses and error messages.
import { kbNote } from "../kbDebug/kbDebug";

export type PushDiag = {
  relayInfo?: string;
  gateway?: string;
  lease?: string;
  leaseAt?: number;
  pushes?: number;
  lastPushAt?: number;
};

const state: PushDiag = {};
const listeners = new Set<() => void>();

export function diag(): Readonly<PushDiag> {
  return state;
}
export function setDiag(patch: Partial<PushDiag>, log?: string) {
  Object.assign(state, patch);
  if (log) kbNote(`push ${log}`);
  for (const f of listeners) f();
}
export function subscribeDiag(f: () => void): () => void {
  listeners.add(f);
  return () => void listeners.delete(f);
}
export const errMsg = (e: unknown) =>
  (e instanceof Error ? e.message : String(e)).slice(0, 160);
