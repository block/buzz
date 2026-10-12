// Web side of "Pass thread context to Buzz Term": what the picker lists (from the relay-bridge snapshot + heartbeat of the
// member's own desktop), what happens after the desktop acks, and the one-shot hand-over to the Buzz term page.
// Pure logic + a tiny store; no React.
import type { AgentSummary, AgentsSnapshotBody, HeartbeatBody, TermHandoffHarness } from "./protocol";
import { whyTermUnavailable, type Reach, type RemoteHost } from "./remoteHosts";

export const HARNESS_LABEL: Record<TermHandoffHarness, string> = { claude: "Claude", codex: "Codex" };
export const NO_DESKTOP_AGENTS_LABEL = "No running agents in this channel on your desktop";

/** Running agents on the desktop that are members of this channel (same policy as the desktop card). */
export function handoffAgentsFor(snapshot: AgentsSnapshotBody | null, channelId: string): AgentSummary[] {
  return (snapshot?.agents ?? []).filter((a) => a.status === "running" && (a.channelIds ?? []).includes(channelId));
}

export type HarnessOption = { id: TermHandoffHarness; label: string; available: boolean; hint: string | null };
/** Harnesses the desktop advertises. null = the desktop hasn't said (older build, or its catalog hasn't loaded yet). */
export function handoffHarnesses(heartbeat: Pick<HeartbeatBody, "harnesses"> | null): HarnessOption[] | null {
  const list = heartbeat?.harnesses;
  if (!list) return null;
  return (Object.keys(HARNESS_LABEL) as TermHandoffHarness[]).map((id) => {
    const available = list.some((h) => h.id === id && h.available);
    return {
      id,
      label: HARNESS_LABEL[id],
      available,
      hint: available ? null : `Install the ${HARNESS_LABEL[id]} CLI on your desktop to use this harness.`,
    };
  });
}

export type AttachPlan = { attach: true } | { attach: false; message: string };
/** After the desktop started the session: open it in the web Buzz term page, or say exactly what to enable. */
export function planAttach(host: RemoteHost | undefined, hasRemoteInfo: boolean, reach: Reach): AttachPlan {
  const why = whyTermUnavailable(host, hasRemoteInfo, reach);
  if (host && host.supports.terminal && !host.stale && reach !== "unreachable" && !why) return { attach: true };
  return {
    attach: false,
    message: `The Buzz Term session started on your desktop. ${
      why ? `To open it here too: ${why}` : "To open it here too, turn on Allow remote terminal on your desktop and connect this device over Tailscale."
    }`,
  };
}

// ── hand-over to the Buzz term page ─────────────────────────────────────────────────────
export type PendingTermHandoff = {
  sid: string;
  name: string;
  tool: "claude" | "codex";
  /** Shell line to type into the remote terminal (reads the prompt file on the desktop). */
  launchCommand: string;
};
/** A hand-over not picked up within this long is dropped, so a later visit to Buzz term never launches a stale session. */
export const PENDING_HANDOFF_TTL_MS = 60_000;
let pending: { h: PendingTermHandoff; at: number } | null = null;
export function setPendingTermHandoff(h: PendingTermHandoff, now = Date.now()) {
  pending = { h, at: now };
}
export function peekPendingTermHandoff(now = Date.now()): PendingTermHandoff | null {
  if (pending && now - pending.at > PENDING_HANDOFF_TTL_MS) pending = null;
  return pending?.h ?? null;
}
export function takePendingTermHandoff(now = Date.now()): PendingTermHandoff | null {
  const h = peekPendingTermHandoff(now);
  pending = null;
  return h;
}

/** Reads the typed-ack result of a term.handoff command. */
export function parseHandoffResult(r: Record<string, unknown> | undefined): PendingTermHandoff | null {
  if (!r) return null;
  const { sid, name, tool, launchCommand } = r;
  if (typeof sid !== "string" || !sid || typeof launchCommand !== "string" || !launchCommand) return null;
  if (tool !== "claude" && tool !== "codex") return null;
  return { sid, name: typeof name === "string" ? name : "Buzz Term session", tool, launchCommand };
}

/**
 * Where the real desktop Buzz Term renderer will plug in: the Buzz term page calls this with the opened remote terminal
 * and the handoff. Today the terminal is the basic text view, so it types the launch line into the new shell; when the
 * renderer is swapped in, replace the body with "attach to session `sid`" on the same TerminalHandle.
 */
export function attachHandoffToTerminal(
  term: { write(s: string): void },
  h: PendingTermHandoff,
  schedule: (fn: () => void, ms: number) => unknown = setTimeout,
) {
  schedule(() => term.write(`${h.launchCommand}\r`), 700); // let the shell print its prompt first
}
