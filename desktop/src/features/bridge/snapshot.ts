// Pure projection: desktop agent state -> bridge AgentSummary[]. Never include secrets, env, prompts, paths or commands.
import type { AgentSummary, AgentsSnapshotBody } from "./protocol";
import { MAX_AGENT_CHANNEL_IDS, MAX_SNAPSHOT_AGENTS } from "./protocol";

export type AgentInput = {
  pubkey: string;
  name: string;
  status: AgentSummary["status"];
  pid?: number | null;
  runtime?: string | null;
  model?: string | null;
  personaId?: string | null;
  lastStartedAt?: string | null;
  lastError?: string | null;
  /** Channels the agent is a member of. */
  channelIds?: string[];
};

export type ChannelTurnInput = {
  channelId: string;
  anchorAt: number;
  agentPubkeys: string[];
};

const norm = (p: string) => p.toLowerCase();
const clip = (s: string, n: number) =>
  s.length > n ? `${s.slice(0, n - 1)}…` : s;

export function buildAgentsSnapshot(
  agents: AgentInput[],
  channelTurns: ChannelTurnInput[],
  channelName: (id: string) => string | undefined,
  now: number,
): AgentsSnapshotBody {
  const turnsByAgent = new Map<string, AgentSummary["activeTurns"]>();
  for (const ct of channelTurns)
    for (const pk of ct.agentPubkeys) {
      const list = turnsByAgent.get(norm(pk)) ?? [];
      list.push({
        channelId: ct.channelId,
        channelName: channelName(ct.channelId),
        since: Math.floor(ct.anchorAt / 1000),
      });
      turnsByAgent.set(norm(pk), list);
    }
  const rank = {
    running: 0,
    deployed: 1,
    stopped: 2,
    not_deployed: 3,
  } as const;
  const sorted = [...agents].sort(
    (a, b) =>
      rank[a.status] - rank[b.status] ||
      a.name.localeCompare(b.name) ||
      a.pubkey.localeCompare(b.pubkey),
  );
  const out: AgentSummary[] = sorted.slice(0, MAX_SNAPSHOT_AGENTS).map((a) => {
    const activeTurns = (turnsByAgent.get(norm(a.pubkey)) ?? []).sort((x, y) =>
      x.channelId.localeCompare(y.channelId),
    );
    const s: AgentSummary = {
      pubkey: a.pubkey,
      name: clip(a.name, 80),
      status: a.status,
      activeTurns,
    };
    if (a.pid != null) s.pid = a.pid;
    if (a.runtime) s.runtime = a.runtime;
    if (a.model) s.model = a.model;
    if (a.personaId) s.personaId = a.personaId;
    if (a.lastStartedAt) s.lastStartedAt = a.lastStartedAt;
    if (a.lastError) s.lastError = clip(a.lastError, 200);
    if ((a.status === "running" || a.status === "deployed") && a.channelIds && a.channelIds.length > 0)
      s.channelIds = [...new Set(a.channelIds)].sort().slice(0, MAX_AGENT_CHANNEL_IDS);
    if (activeTurns.length > 0) {
      const names = activeTurns.map((t) => t.channelName ?? "a channel");
      s.currentTask = clip(
        `Working in ${names.slice(0, 2).join(", ")}${names.length > 2 ? ` +${names.length - 2}` : ""}`,
        140,
      );
    }
    return s;
  });
  return {
    agents: out,
    ...(agents.length > out.length ? { truncated: true } : {}),
    generatedAt: now,
  };
}

/** Stable content signature (excludes generatedAt and turn start times) so unchanged state is not republished. */
export function snapshotSignature(b: AgentsSnapshotBody): string {
  return JSON.stringify(
    b.agents.map((a) => [
      a.pubkey,
      a.name,
      a.status,
      a.pid ?? null,
      a.model ?? null,
      a.runtime ?? null,
      a.lastError ?? null,
      a.activeTurns.map((t) => t.channelId),
      a.currentTask ?? null,
      a.channelIds ?? null,
    ]),
  );
}
