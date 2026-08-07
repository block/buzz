import type { ManagedAgentBackend } from "@/shared/api/types";

import { summarizeRunOn } from "./runOnSummary";

/**
 * Read-only "Run on" summary for the edit-agent dialog.
 *
 * Read-only on purpose: `UpdateManagedAgentRequest` has no backend field —
 * where an agent runs is fixed at creation (a provider-backed agent's
 * deployment holds its private key; there is no migrate operation). This
 * section shows the *saved* provider config from the record, without probing
 * the provider binary: an edit dialog must not do executable work as a side
 * effect, and a live probe would show today's schema defaults instead of
 * what this agent actually deployed with.
 *
 * Named "Run on" (matching the create flow) rather than "Provider" because
 * this dialog already uses "Provider" for the ACP harness selector.
 */
export function RunOnSummarySection({
  backend,
}: {
  backend: ManagedAgentBackend;
}) {
  const summary = summarizeRunOn(backend);

  return (
    <div className="space-y-1.5" data-testid="edit-agent-run-on">
      <span className="text-sm font-medium text-foreground">Run on</span>
      {summary.location === "local" ? (
        <p
          className="text-sm text-muted-foreground"
          data-testid="edit-agent-run-on-location"
        >
          This computer
        </p>
      ) : (
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
      )}
      <p className="text-xs text-muted-foreground">
        These are the settings saved when the agent was created. Where an agent
        runs can&apos;t be changed afterwards — create a new agent to run
        somewhere else.
      </p>
    </div>
  );
}
