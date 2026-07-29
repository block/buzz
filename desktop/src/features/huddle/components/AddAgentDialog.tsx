import { Bot } from "lucide-react";
import * as React from "react";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { huddleAgentPicks } from "@/features/huddle/lib/huddleAgentPicks";
import { usePresenceQuery } from "@/features/presence/hooks";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

type AgentAddResult = {
  ephemeral_added: boolean;
  parent_added: boolean;
  parent_error: string | null;
};

type AddAgentDialogProps = {
  onClose: () => void;
  onAdd: (pubkey: string) => Promise<AgentAddResult>;
  currentAgentPubkeys: string[];
};

export function AddAgentDialog({
  onClose,
  onAdd,
  currentAgentPubkeys,
}: AddAgentDialogProps) {
  const [adding, setAdding] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [warning, setWarning] = React.useState<string | null>(null);

  const agentsQuery = useManagedAgentsQuery();
  const agents = React.useMemo(
    () => agentsQuery.data ?? [],
    [agentsQuery.data],
  );
  const presenceQuery = usePresenceQuery(agents.map((a) => a.pubkey));
  // Presence decides eligibility for remote records, so a list rendered before
  // it lands would flash "no online agents" at a live fleet. The query is
  // disabled (never loading) when there are no pubkeys to look up.
  const loading = agentsQuery.isLoading || presenceQuery.isLoading;
  const loadError = agentsQuery.error ? "Could not load agents." : null;

  const { picks, emptyMessage } = React.useMemo(
    () =>
      huddleAgentPicks({
        agents,
        presenceLookup: presenceQuery.data,
        currentAgentPubkeys,
      }),
    [agents, presenceQuery.data, currentAgentPubkeys],
  );

  async function handleAdd(pubkey: string) {
    if (adding) return;
    setAdding(pubkey);
    setError(null);
    setWarning(null);
    try {
      const result = await onAdd(pubkey);
      if (result.parent_error) {
        // Agent was added to the ephemeral channel but parent channel add failed.
        // Show as a warning — don't close the dialog so the user can see it.
        setWarning(
          `Added to huddle, but parent channel failed: ${result.parent_error}`,
        );
      } else {
        onClose();
      }
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(`Failed to add agent: ${msg}`);
      console.error("Failed to add agent to huddle:", e);
    } finally {
      setAdding(null);
    }
  }

  return (
    <Dialog
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      open
    >
      <DialogContent className="flex max-h-[60vh] max-w-sm flex-col gap-0 p-0">
        <DialogHeader className="border-b px-6 py-4">
          <DialogTitle>Add Agent to Huddle</DialogTitle>
          <DialogDescription>
            Select an online agent to join the huddle.
          </DialogDescription>
        </DialogHeader>

        <div className="flex-1 overflow-y-auto px-6 py-4">
          {(error ?? loadError) && (
            <p className="mb-3 rounded-md bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {error ?? loadError}
            </p>
          )}

          {warning && (
            <div className="mb-3 flex items-start justify-between gap-2 rounded-md bg-amber-500/10 px-3 py-2 text-sm text-amber-700 dark:text-amber-400">
              <span>{warning}</span>
              <button
                className="shrink-0 font-medium underline-offset-2 hover:underline"
                onClick={onClose}
                type="button"
              >
                Dismiss
              </button>
            </div>
          )}

          {loading ? (
            <p className="py-4 text-center text-sm text-muted-foreground">
              Loading agents…
            </p>
          ) : picks.length === 0 ? (
            <p className="py-4 text-center text-sm text-muted-foreground">
              {emptyMessage}
            </p>
          ) : (
            <ul className="flex flex-col gap-1">
              {picks.map((agent) => (
                <li key={agent.pubkey}>
                  <button
                    className="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-sm transition-colors hover:bg-accent hover:text-accent-foreground disabled:opacity-50"
                    disabled={adding === agent.pubkey}
                    onClick={() => void handleAdd(agent.pubkey)}
                    type="button"
                  >
                    <Bot className="h-4 w-4 shrink-0 text-muted-foreground" />
                    <span className="flex-1 truncate font-medium">
                      {agent.name}
                    </span>
                    <span className="shrink-0 text-xs text-muted-foreground">
                      {agent.presence}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>

        <div className="border-t px-6 py-4">
          <Button className="w-full" onClick={onClose} variant="outline">
            Cancel
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

export type { AgentAddResult };
