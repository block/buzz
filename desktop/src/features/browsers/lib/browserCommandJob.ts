// Progress tracking for one web -> desktop browser command (pure, unit-tested).
// Phases: sending (signing/publishing) -> waiting (sent, no ack yet) -> accepted (desktop is working) -> done | failed | timeout.
export type JobPhase = "sending" | "waiting" | "accepted" | "done" | "failed" | "timeout";

export const ACK_TIMEOUT_MS = 30_000;
export const FINISH_TIMEOUT_MS = 60_000;

export type JobInput = {
  sentAtMs: number | null;
  /** First time the desktop's "accepted" ack was seen. */
  acceptedAtMs: number | null;
  /** Result status as reported by the bridge (null = nothing yet). */
  result: "accepted" | "ok" | "error" | null;
  nowMs: number;
};

export function jobPhase(i: JobInput): JobPhase {
  if (i.result === "ok") return "done";
  if (i.result === "error") return "failed";
  if (i.sentAtMs === null) return "sending";
  if (i.result === "accepted") {
    return i.nowMs - (i.acceptedAtMs ?? i.sentAtMs) > FINISH_TIMEOUT_MS ? "timeout" : "accepted";
  }
  return i.nowMs - i.sentAtMs > ACK_TIMEOUT_MS ? "timeout" : "waiting";
}

export function phaseText(phase: JobPhase, label: string, message?: string): string {
  switch (phase) {
    case "sending":
      return `${label}: sending… (your signer may ask you to approve)`;
    case "waiting":
      return `${label}: sent, waiting for your desktop…`;
    case "accepted":
      return `${label}: your desktop accepted it, working…`;
    case "done":
      return `${label}: done`;
    case "failed":
      return `${label}: failed${message ? ` — ${message}` : ""}`;
    case "timeout":
      return `${label}: your desktop didn't respond. Is it online with "Allow remote agent management" on? Reload the desktop app, then retry.`;
  }
}
