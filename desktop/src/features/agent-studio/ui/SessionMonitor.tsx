import * as React from "react";
import { Activity } from "lucide-react";

import { useCommunities } from "@/features/communities/useCommunities";

type SessionRow = {
  session_id: string;
  agent_id?: string | null;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
  tool_calls: number;
};

type SessionMonitorProps = {
  /** When true, omit outer section chrome (used inside UnifiedCostMonitor). */
  embedded?: boolean;
};

export function SessionMonitor({ embedded = false }: SessionMonitorProps) {
  const { activeCommunity } = useCommunities();
  const [sessions, setSessions] = React.useState<SessionRow[]>([]);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let cancelled = false;
    const relayHttp = activeCommunity?.relayUrl?.replace(/^ws/i, "http");
    if (!relayHttp) return;

    const load = () => {
      void fetch(`${relayHttp}/agent-studio/sessions`)
        .then((res) => {
          if (!res.ok) throw new Error(`HTTP ${res.status}`);
          return res.json();
        })
        .then((data: { sessions?: SessionRow[] }) => {
          if (!cancelled) {
            setSessions(data.sessions ?? []);
            setError(null);
          }
        })
        .catch((e: unknown) => {
          if (!cancelled) {
            setError(
              e instanceof Error ? e.message : "Failed to load sessions",
            );
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

  const totalCost = sessions.reduce((sum, s) => sum + s.cost_usd, 0);
  const totalTokens = sessions.reduce(
    (sum, s) => sum + s.input_tokens + s.output_tokens,
    0,
  );

  const body = (
    <>
      {error ? <p className="text-sm text-red-400">{error}</p> : null}
      {sessions.length === 0 && !error ? (
        <p className="text-sm text-muted-foreground">No active sessions.</p>
      ) : null}
      {sessions.length > 0 ? (
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
            {sessions.map((s) => (
              <tr key={s.session_id}>
                <td className="py-1 font-mono text-2xs">{s.session_id}</td>
                <td className="py-1">{s.agent_id ?? "—"}</td>
                <td className="py-1">{s.input_tokens + s.output_tokens}</td>
                <td className="py-1">${s.cost_usd.toFixed(4)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}
    </>
  );

  if (embedded) {
    return (
      <div className="rounded-lg border border-border bg-card p-4">
        <div className="mb-3 flex items-center gap-2 text-sm font-medium">
          <Activity className="h-4 w-4" />
          Sessions
          <span className="text-2xs font-normal text-muted-foreground">
            ${totalCost.toFixed(4)} · {totalTokens.toLocaleString()} tokens
          </span>
        </div>
        {body}
      </div>
    );
  }

  return (
    <section className="mt-6 rounded-lg border border-border bg-card p-4">
      <div className="mb-3 flex items-center gap-2 text-sm font-medium">
        <Activity className="h-4 w-4" />
        Session monitor
        <span className="text-2xs font-normal text-muted-foreground">
          ${totalCost.toFixed(4)} total
        </span>
      </div>
      {body}
    </section>
  );
}
