import * as React from "react";
import { Loader2, Plug, RefreshCw, Server, Unplug } from "lucide-react";

import { probeAgentHost } from "@/shared/api/remoteAgentApi";
import type {
  ConnectedAgent,
  HostProbeResult,
} from "@/shared/api/remoteAgentTypes";
import { Button } from "@/shared/ui/button";
import { SectionHeader } from "@/shared/ui/PageHeader";
import { reachabilityLabel } from "./connectAgentIntent";
import { PubKey } from "@/shared/ui/PubKey";

/**
 * The Connected-agents surface: agents that run on machines the user owns.
 *
 * There are deliberately no Start, Stop, Restart, or Deploy controls anywhere
 * in this section. Buzz does not own these processes, and a button that cannot
 * work is worse than no button — it invites the user to conclude the agent is
 * broken when it is simply not Buzz's to command. The only actions offered are
 * the two Buzz can actually perform: check whether the machine answers, and
 * forget the agent locally.
 */
export function ConnectedAgentsSection({
  agents,
  error,
  isLoading,
  isPending,
  noticeMessage,
  onConnect,
  onDisconnect,
}: {
  agents: ConnectedAgent[];
  error: Error | null;
  isLoading: boolean;
  isPending: boolean;
  noticeMessage: string | null;
  onConnect: () => void;
  onDisconnect: (agent: ConnectedAgent) => void;
}) {
  const [probes, setProbes] = React.useState<
    Record<string, HostProbeResult | "pending">
  >({});

  const checkHost = React.useCallback((host: string) => {
    setProbes((current) => ({ ...current, [host]: "pending" }));
    void probeAgentHost(host)
      .then((result) => {
        setProbes((current) => ({ ...current, [host]: result }));
      })
      .catch((cause) => {
        // A failure to run ssh at all is still a reachability answer; render it
        // rather than leaving the row stuck on "checking".
        setProbes((current) => ({
          ...current,
          [host]: {
            host,
            ok: false,
            durationMs: 0,
            error: cause instanceof Error ? cause.message : String(cause),
            harnesses: [],
          },
        }));
      });
  }, []);

  return (
    <section
      className="relative space-y-4"
      data-testid="agents-library-connected"
    >
      <SectionHeader
        action={
          <Button
            disabled={isPending}
            onClick={onConnect}
            size="sm"
            variant="outline"
          >
            <Plug />
            Connect an agent
          </Button>
        }
        className="mx-auto w-full max-w-[996px]"
        description="Agents running on your own machines. They hold their own keys and start themselves — Buzz talks to them."
        title="Connected agents"
      />

      <div className="mx-auto w-full max-w-[996px] space-y-2">
        {error ? (
          <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            Could not load connected agents: {error.message}
          </p>
        ) : null}

        {noticeMessage ? (
          <p className="rounded-2xl border border-border bg-muted/30 px-4 py-3 text-sm text-muted-foreground">
            {noticeMessage}
          </p>
        ) : null}

        {isLoading ? (
          <p className="text-sm text-muted-foreground">Loading…</p>
        ) : null}

        {!isLoading && agents.length === 0 && !error ? (
          <p className="rounded-2xl border border-border bg-muted/30 px-4 py-3 text-sm text-muted-foreground">
            Nothing connected yet. An agent on another machine needs its own key
            and the <span className="font-mono">buzz</span> CLI; connect it here
            once it can reach the relay.
          </p>
        ) : null}

        {agents.map((agent) => (
          <ConnectedAgentRow
            agent={agent}
            isPending={isPending}
            key={agent.pubkey}
            onCheck={() => checkHost(agent.host)}
            onDisconnect={() => onDisconnect(agent)}
            probe={probes[agent.host]}
          />
        ))}
      </div>
    </section>
  );
}

function ConnectedAgentRow({
  agent,
  isPending,
  onCheck,
  onDisconnect,
  probe,
}: {
  agent: ConnectedAgent;
  isPending: boolean;
  onCheck: () => void;
  onDisconnect: () => void;
  probe: HostProbeResult | "pending" | undefined;
}) {
  return (
    <div className="flex flex-wrap items-center gap-3 rounded-2xl border border-border px-4 py-3">
      <Server className="h-4 w-4 shrink-0 text-muted-foreground" />
      <div className="min-w-0 flex-1 space-y-0.5">
        <div className="flex flex-wrap items-baseline gap-x-2">
          <span className="font-medium">{agent.name}</span>
          <span className="text-sm text-muted-foreground">
            on <span className="font-mono">{agent.host}</span>
          </span>
          {agent.harness ? (
            <span className="text-2xs text-muted-foreground">
              {agent.harness}
            </span>
          ) : null}
        </div>
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
          <PubKey pubkey={agent.pubkey} />
          <Reachability probe={probe} />
        </div>
      </div>
      <div className="flex shrink-0 gap-2">
        <Button
          aria-label={`Check ${agent.host}`}
          disabled={probe === "pending"}
          onClick={onCheck}
          size="sm"
          type="button"
          variant="ghost"
        >
          {probe === "pending" ? (
            <Loader2 className="animate-spin" />
          ) : (
            <RefreshCw />
          )}
          Check
        </Button>
        <Button
          aria-label={`Disconnect ${agent.name}`}
          disabled={isPending}
          onClick={onDisconnect}
          size="sm"
          type="button"
          variant="ghost"
        >
          <Unplug />
          Disconnect
        </Button>
      </div>
    </div>
  );
}

/**
 * Reachability of the machine, not liveness of the agent.
 *
 * The distinction is deliberate and the wording keeps it: a reachable host does
 * not mean the agent process is up, and Buzz has no way to ask. Presence on the
 * relay — which the agent publishes itself — is the answer to "is it running",
 * and it belongs to the agent, not to this panel.
 */
function Reachability({
  probe,
}: {
  probe: HostProbeResult | "pending" | undefined;
}) {
  if (probe === undefined) return null;
  if (probe === "pending") {
    return <span className="text-2xs text-muted-foreground">checking…</span>;
  }
  if (probe.ok) {
    return (
      <span className="text-2xs text-muted-foreground">
        machine reachable
        {probe.buzzCliPath ? "" : " · no buzz CLI"}
      </span>
    );
  }
  return (
    <span className="text-2xs text-warning">{reachabilityLabel(probe)}</span>
  );
}
