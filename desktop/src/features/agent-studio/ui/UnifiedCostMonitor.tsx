import * as React from "react";
import { DollarSign } from "lucide-react";

import { useCommunities } from "@/features/communities/useCommunities";

type CostSummary = {
  total_cost_usd: number;
  acp_session_cost_usd: number;
  flow_block_cost_usd: number;
  total_tokens: number;
  session_count: number;
  sessions: Array<{
    session_id: string;
    agent_id?: string | null;
    input_tokens: number;
    output_tokens: number;
    cost_usd: number;
  }>;
};

/** Unified cost dashboard — ACP sessions plus Flow block execution costs. */
export function UnifiedCostMonitor() {
  const { activeCommunity } = useCommunities();
  const [summary, setSummary] = React.useState<CostSummary | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let cancelled = false;
    const relayHttp = activeCommunity?.relayUrl?.replace(/^ws/i, "http");
    if (!relayHttp) return;

    const load = () => {
      void fetch(`${relayHttp}/agent-studio/costs`)
        .then((res) => {
          if (!res.ok) throw new Error(`HTTP ${res.status}`);
          return res.json();
        })
        .then((data: CostSummary) => {
          if (!cancelled) {
            setSummary(data);
            setError(null);
          }
        })
        .catch((e: unknown) => {
          if (!cancelled) {
            setError(e instanceof Error ? e.message : "Failed to load costs");
          }
        });
    };

    load();
    const id = window.setInterval(load, 5000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [activeCommunity?.relayUrl]);

  return (
    <section className="mt-6 rounded-lg border border-border bg-card p-4">
      <div className="mb-3 flex flex-wrap items-center gap-2 text-sm font-medium">
        <DollarSign className="h-4 w-4" />
        Cost monitor
        {summary ? (
          <span className="text-2xs font-normal text-muted-foreground">
            ${summary.total_cost_usd.toFixed(4)} total ·{" "}
            {summary.total_tokens.toLocaleString()} tokens ·{" "}
            {summary.session_count} sessions
          </span>
        ) : null}
      </div>
      {summary ? (
        <div className="mb-3 grid gap-2 text-sm sm:grid-cols-2">
          <p className="text-muted-foreground">
            ACP sessions: ${summary.acp_session_cost_usd.toFixed(4)}
          </p>
          <p className="text-muted-foreground">
            Flow blocks: ${summary.flow_block_cost_usd.toFixed(4)}
          </p>
        </div>
      ) : null}
      {error ? <p className="text-sm text-red-400">{error}</p> : null}
      {!summary && !error ? (
        <p className="text-sm text-muted-foreground">Loading costs…</p>
      ) : null}
      {summary && summary.sessions.length > 0 ? (
        <table className="w-full text-sm">
          <thead>
            <tr className="text-left text-2xs text-muted-foreground">
              <th className="pb-2">Session</th>
              <th className="pb-2">Agent</th>
              <th className="pb-2">Tokens</th>
              <th className="pb-2">Cost</th>
            </tr>
          </thead>
          <tbody>
            {summary.sessions.map((session) => (
              <tr key={session.session_id}>
                <td className="py-1 font-mono text-2xs">
                  {session.session_id}
                </td>
                <td className="py-1">{session.agent_id ?? "—"}</td>
                <td className="py-1">
                  {session.input_tokens + session.output_tokens}
                </td>
                <td className="py-1">${session.cost_usd.toFixed(4)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : summary ? (
        <p className="text-sm text-muted-foreground">No active sessions.</p>
      ) : null}
    </section>
  );
}
