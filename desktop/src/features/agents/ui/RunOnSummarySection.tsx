import type { ManagedAgentBackend } from "@/shared/api/types";

import { summarizeRunOn } from "./runOnSummary";

/**
 * Read-only summary of a provider agent's *saved* run-on settings, rendered
 * beneath the interactive "Run on" picker in the edit dialog.
 *
 * Only provider backends render: local and execution-node agents have no
 * saved config rows (node runtime details deliberately stay on the node),
 * and the picker already names the location. The rows come from the record,
 * without probing the provider binary: an edit dialog must not do executable
 * work as a side effect, and a live probe would show today's schema defaults
 * instead of what this agent actually deployed with.
 */
export function RunOnSummarySection({
  backend,
}: {
  backend: ManagedAgentBackend;
}) {
  const summary = summarizeRunOn(backend);
  if (summary.location !== "provider") return null;

  return (
    <div className="space-y-1.5" data-testid="edit-agent-run-on">
      <span className="text-sm font-medium text-foreground">
        Saved run-on settings
      </span>
      <div className="space-y-2 rounded-2xl border border-border bg-muted/30 px-4 py-3">
        <p
          className="text-sm font-medium"
          data-testid="edit-agent-run-on-location"
        >
          {summary.providerId}
        </p>
        {summary.rows.length > 0 ? (
          <dl className="space-y-1">
            {summary.rows.map((row) => (
              <div
                className="flex flex-wrap items-baseline gap-x-3 gap-y-0.5"
                data-testid={`edit-agent-run-on-${row.key}`}
                key={row.key}
              >
                <dt className="text-xs text-muted-foreground">{row.label}</dt>
                <dd className="min-w-0 break-all font-mono text-xs text-foreground">
                  {row.value}
                </dd>
              </div>
            ))}
          </dl>
        ) : (
          <p className="text-xs text-muted-foreground">
            No saved settings — the provider applies its defaults.
          </p>
        )}
      </div>
      <p className="text-xs text-muted-foreground">
        These are the settings saved when the agent was created. Picking a
        different Run on target above redeploys the agent there.
      </p>
    </div>
  );
}
