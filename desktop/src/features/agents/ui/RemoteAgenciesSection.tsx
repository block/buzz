import * as React from "react";
import { Network, Plus } from "lucide-react";

import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { listRemoteAgencies } from "@/shared/api/tauri";
import type { RemoteAgencyBinding } from "@/shared/api/remoteAgencyTypes";
import type { ManagedAgent } from "@/shared/api/types";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { RemoteAgencyDialog } from "./RemoteAgencyDialog";

const REMOTE_AGENT_NAME_PREFIX = "Remote Agency · proxied by Buzz · ";

function remoteDisplayName(agent: ManagedAgent) {
  return agent.name.startsWith(REMOTE_AGENT_NAME_PREFIX)
    ? agent.name.slice(REMOTE_AGENT_NAME_PREFIX.length)
    : agent.name;
}

/** Entry point for importing an existing agent organization into Buzz. */
export function RemoteAgenciesSection({ agents }: { agents: ManagedAgent[] }) {
  const [open, setOpen] = React.useState(false);
  const [bindings, setBindings] = React.useState<RemoteAgencyBinding[]>([]);
  const refreshBindings = React.useCallback(() => {
    void listRemoteAgencies()
      .then(setBindings)
      .catch(() => setBindings([]));
  }, []);
  React.useEffect(refreshBindings, [refreshBindings]);
  const remotePubkeys = React.useMemo(
    () =>
      new Set(
        bindings.flatMap((binding) =>
          binding.proxies.map((proxy) => proxy.pubkey),
        ),
      ),
    [bindings],
  );
  const remoteAgents = agents.filter((agent) =>
    remotePubkeys.has(agent.pubkey),
  );

  return (
    <section className="space-y-3" data-testid="remote-agencies-section">
      <div className="flex items-center justify-between gap-3">
        <div>
          <h2 className="flex items-center gap-2 text-base font-semibold">
            <Network className="size-4 text-primary" />
            Remote Agencies
          </h2>
          <p className="mt-1 text-sm text-muted-foreground">
            Join an existing Agency manifest. Agent records use OASF, and
            invocation uses A2A.
          </p>
        </div>
        <Button onClick={() => setOpen(true)} size="sm" variant="outline">
          <Plus />
          Add Remote Agency
        </Button>
      </div>
      {remoteAgents.length === 0 ? (
        <div className="rounded-xl border border-dashed border-border/70 bg-muted/10 px-4 py-5 text-sm text-muted-foreground">
          Remote participants appear here and in your selected channel after
          review. Buzz uses a local proxy identity for each participant.
        </div>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {remoteAgents.map((agent) => {
            const displayName = remoteDisplayName(agent);
            const connected = isManagedAgentActive(agent);
            return (
              <article
                className="rounded-xl border border-border/60 bg-card p-4"
                key={agent.pubkey}
              >
                <div className="flex items-start gap-3">
                  <UserAvatar
                    accent
                    avatarUrl={agent.avatarUrl}
                    displayName={displayName}
                  />
                  <div className="min-w-0 flex-1">
                    <h3 className="truncate font-medium">{displayName}</h3>
                    <p className="mt-0.5 text-xs text-muted-foreground">
                      Existing Agent · remote runtime
                    </p>
                  </div>
                  <span
                    aria-hidden
                    className={`mt-1 size-2 rounded-full ${
                      connected ? "bg-success" : "bg-muted-foreground/50"
                    }`}
                  />
                  <span className="sr-only">
                    {connected ? "Proxy running" : "Proxy stopped"}
                  </span>
                </div>
                <div className="mt-4 flex flex-wrap gap-1.5">
                  <Badge variant="secondary">Remote</Badge>
                  <Badge variant="outline">OASF record</Badge>
                  <Badge variant="outline">A2A configured</Badge>
                </div>
              </article>
            );
          })}
        </div>
      )}
      <RemoteAgencyDialog
        onBindingChange={refreshBindings}
        onOpenChange={setOpen}
        open={open}
      />
    </section>
  );
}
