import * as React from "react";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import {
  useManagedAgentRuntimeAction,
  useManagedAgentRuntimesQuery,
} from "@/features/agents/managedAgentRuntimeHooks";
import {
  agentCommunityAvailability,
  agentCommunityStatusDetail,
  managedAgentRuntimeKey,
} from "@/features/agents/managedAgentRuntimeStatus";
import type { ManagedAgentRuntimeStatus } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { Badge } from "@/shared/ui/badge";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

export function ActiveAgentCommunitiesSettingsCard() {
  const agentsQuery = useManagedAgentsQuery();
  const runtimesQuery = useManagedAgentRuntimesQuery();
  const action = useManagedAgentRuntimeAction();
  const [pendingRuntimeKey, setPendingRuntimeKey] = React.useState<
    string | null
  >(null);

  const agentNames = React.useMemo(
    () =>
      new Map(
        (agentsQuery.data ?? []).map((agent) => [
          agent.pubkey.toLowerCase(),
          agent.name,
        ]),
      ),
    [agentsQuery.data],
  );
  const runtimes = runtimesQuery.data ?? [];

  async function runAction(runtime: ManagedAgentRuntimeStatus) {
    setPendingRuntimeKey(managedAgentRuntimeKey(runtime));
    try {
      await action.mutateAsync({
        action:
          runtime.lifecycle === "starting" ||
          runtime.lifecycle === "listening" ||
          runtime.lifecycle === "waking" ||
          runtime.lifecycle === "ready"
            ? "stop"
            : runtime.lifecycle === "stopped"
              ? "start"
              : "restart",
        pubkey: runtime.pubkey,
        relayUrl: runtime.relayUrl,
      });
    } finally {
      setPendingRuntimeKey(null);
    }
  }

  return (
    <section className="min-w-0" data-testid="active-agent-communities">
      <SettingsSectionHeader
        title="Active in communities"
        description="See and control each community where this device runs your agents."
      />
      <div className="overflow-hidden rounded-xl border border-border/60">
        {runtimesQuery.isPending ? (
          <p className="px-4 py-3 text-sm text-muted-foreground">Loading…</p>
        ) : runtimes.length === 0 ? (
          <p className="px-4 py-3 text-sm text-muted-foreground">
            No agent community runtimes found.
          </p>
        ) : (
          runtimes.map((runtime) => {
            const status = agentCommunityAvailability(runtime);
            const detail = agentCommunityStatusDetail(runtime);
            const runtimeKey = managedAgentRuntimeKey(runtime);
            const pending = pendingRuntimeKey === runtimeKey;
            return (
              <div
                className="flex items-center gap-3 border-b border-border/60 px-4 py-3 last:border-b-0"
                data-testid="agent-community-runtime"
                key={runtimeKey}
              >
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <p className="text-sm font-medium">
                      {agentNames.get(runtime.pubkey.toLowerCase()) ??
                        truncatePubkey(runtime.pubkey)}
                    </p>
                    <Badge
                      variant={status === "Here" ? "default" : "secondary"}
                    >
                      {status}
                    </Badge>
                  </div>
                  <p className="truncate text-xs text-muted-foreground">
                    {runtime.relayUrl}
                  </p>
                  {detail ? (
                    <p className="text-xs text-muted-foreground">{detail}</p>
                  ) : null}
                </div>
                {runtime.localSetup ? (
                  <Button
                    disabled={pending}
                    onClick={() => void runAction(runtime)}
                    size="sm"
                    type="button"
                    variant="outline"
                  >
                    {pending
                      ? "Working…"
                      : runtime.lifecycle === "stopped"
                        ? "Start"
                        : runtime.lifecycle === "failed"
                          ? "Restart"
                          : "Stop"}
                  </Button>
                ) : null}
              </div>
            );
          })
        )}
      </div>
      {action.error instanceof Error ? (
        <p className="mt-2 text-sm text-destructive">{action.error.message}</p>
      ) : null}
    </section>
  );
}
