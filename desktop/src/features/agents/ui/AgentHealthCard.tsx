import { AlertTriangle, CheckCircle2, CircleHelp } from "lucide-react";

import { buildAgentHealthSnapshot } from "@/features/agents/lib/agentHealth";
import type { ManagedAgent, PresenceStatus } from "@/shared/api/types";
import type { ProfileChannelLink } from "@/features/profile/ui/UserProfilePanelUtils";
import { Badge } from "@/shared/ui/badge";
import { UserAvatar } from "@/shared/ui/UserAvatar";

export function AgentHealthCard({
  agent,
  channels,
  channelsError,
  channelsLoading,
  presenceLoaded,
  presenceStatus,
}: {
  agent: ManagedAgent;
  channels: ProfileChannelLink[];
  channelsError: boolean;
  channelsLoading: boolean;
  presenceLoaded: boolean;
  presenceStatus: PresenceStatus | undefined;
}) {
  const snapshot = buildAgentHealthSnapshot({
    agent,
    channels,
    channelsError,
    channelsLoading,
    presenceLoaded,
    presenceStatus,
  });

  return (
    <section className="space-y-3" data-testid="agent-health-card">
      <div>
        <h3 className="text-sm font-semibold text-foreground">Agent health</h3>
        <p className="mt-1 text-xs leading-5 text-muted-foreground">
          Owner-only configuration and runtime facts. Unknown and unavailable
          values are never inferred.
        </p>
      </div>

      {snapshot.warnings.length > 0 ? (
        <div
          className="space-y-2 rounded-2xl border border-amber-500/30 bg-amber-500/5 p-3"
          data-testid="agent-health-warnings"
        >
          <p className="text-xs font-medium text-foreground">
            Configuration warnings
          </p>
          {snapshot.warnings.map((warning) => (
            <div className="flex items-start gap-2" key={warning.key}>
              <AlertTriangle
                className={
                  warning.severity === "error"
                    ? "mt-0.5 h-4 w-4 shrink-0 text-destructive"
                    : "mt-0.5 h-4 w-4 shrink-0 text-amber-600 dark:text-amber-400"
                }
              />
              <p className="text-xs leading-5 text-muted-foreground">
                {warning.label}
              </p>
            </div>
          ))}
        </div>
      ) : (
        <div
          className="flex items-center gap-2 rounded-2xl bg-muted/20 px-4 py-3"
          data-testid="agent-health-no-warnings"
        >
          <CheckCircle2 className="h-4 w-4 text-emerald-500" />
          <p className="text-xs text-muted-foreground">
            No configuration warnings reported.
          </p>
        </div>
      )}

      <div className="overflow-hidden rounded-2xl bg-muted/20">
        {snapshot.fields.map((field) => (
          <div
            className="flex items-start gap-3 border-b border-border/40 px-4 py-3 last:border-b-0"
            data-availability={field.availability}
            data-testid={`agent-health-${field.key}`}
            key={field.key}
          >
            {field.key === "avatar" ? (
              <UserAvatar
                avatarUrl={agent.avatarUrl}
                className="shrink-0"
                displayName={agent.name}
                size="sm"
              />
            ) : (
              <CircleHelp className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
            )}
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-2">
                <p className="text-xs font-medium text-foreground">
                  {field.label}
                </p>
                {field.availability !== "available" ? (
                  <Badge
                    variant={
                      field.availability === "unavailable"
                        ? "secondary"
                        : "outline"
                    }
                  >
                    {field.availability}
                  </Badge>
                ) : null}
              </div>
              <p className="mt-0.5 wrap-break-word text-sm text-muted-foreground">
                {field.value}
              </p>
              {field.detail ? (
                <p className="mt-1 text-xs leading-5 text-muted-foreground">
                  {field.detail}
                </p>
              ) : null}
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}
