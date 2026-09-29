import { SquareTerminal } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import {
  useAcpRuntimesQuery,
  useManagedAgentsQuery,
  useStartManagedAgentMutation,
} from "@/features/agents/hooks";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { normalizePubkey } from "@/shared/lib/pubkey";

import { buildTermSessionHandoffInstruction } from "../lib/handoffInstruction.ts";
import { listTermSessionHarnesses } from "../lib/harnesses.ts";
import {
  filterTermSessionHandoffAgents,
  NO_HANDOFF_AGENTS_LABEL,
} from "../lib/roster.ts";
import type { TermSessionTool } from "../lib/types.ts";

type TermSessionHandoffPopoverProps = {
  channelId: string;
  threadId: string;
  channelMemberPubkeys: readonly string[];
  onSend: (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
    channelId?: string | null,
    threadContext?: {
      parentEventId: string | null;
      threadHeadId: string | null;
    } | null,
  ) => Promise<void>;
};

export function TermSessionHandoffPopover({
  channelId,
  threadId,
  channelMemberPubkeys,
  onSend,
}: TermSessionHandoffPopoverProps) {
  const [open, setOpen] = React.useState(false);
  const [agentPubkey, setAgentPubkey] = React.useState("");
  const [harness, setHarness] = React.useState<TermSessionTool>("claude");
  const [busy, setBusy] = React.useState(false);

  const agentsQuery = useManagedAgentsQuery({ enabled: open });
  const runtimesQuery = useAcpRuntimesQuery({ enabled: open });
  const startMutation = useStartManagedAgentMutation();

  const agents = React.useMemo(
    () =>
      filterTermSessionHandoffAgents(
        agentsQuery.data ?? [],
        channelMemberPubkeys,
      ),
    [agentsQuery.data, channelMemberPubkeys],
  );

  const harnesses = React.useMemo(
    () => listTermSessionHarnesses(runtimesQuery.data ?? []),
    [runtimesQuery.data],
  );

  React.useEffect(() => {
    if (!open) return;
    setAgentPubkey("");
    const firstAvailable =
      harnesses.find((option) => option.available)?.id ?? "claude";
    setHarness(firstAvailable);
  }, [open, harnesses]);

  React.useEffect(() => {
    if (
      agentPubkey &&
      !agents.some((agent) => normalizePubkey(agent.pubkey) === normalizePubkey(agentPubkey))
    ) {
      setAgentPubkey("");
    }
  }, [agents, agentPubkey]);

  const selectedAgent =
    agents.find(
      (agent) =>
        normalizePubkey(agent.pubkey) === normalizePubkey(agentPubkey),
    ) ?? null;
  const selectedHarness =
    harnesses.find((option) => option.id === harness) ?? null;

  const canGo =
    Boolean(selectedAgent) &&
    Boolean(selectedHarness?.available) &&
    !busy;

  async function handleGo() {
    if (!selectedAgent || !selectedHarness?.available) return;
    setBusy(true);
    try {
      if (
        selectedAgent.backend.type === "local" &&
        !isManagedAgentActive(selectedAgent)
      ) {
        try {
          await startMutation.mutateAsync(selectedAgent.pubkey);
        } catch (error) {
          toast.error(
            error instanceof Error
              ? error.message
              : "Start the agent before handing off to Buzz Term.",
          );
          return;
        }
      }

      const content = buildTermSessionHandoffInstruction({
        agentDisplayName: selectedAgent.name || selectedAgent.pubkey.slice(0, 12),
        channelId,
        threadId,
        harness,
        openclawWorkspace: selectedAgent.useOpenClawWorkspace === true,
        mentionPubkey: selectedAgent.pubkey,
      });

      await onSend(
        content,
        [selectedAgent.pubkey],
        undefined,
        channelId,
        { parentEventId: threadId, threadHeadId: threadId },
      );
      setOpen(false);
      toast.message("Asked the agent for a Buzz Term handoff card.");
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Could not send the Buzz Term handoff request.",
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <Popover onOpenChange={setOpen} open={open}>
      <Tooltip disableHoverableContent>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <Button
              aria-label="Pass thread context to Buzz Term"
              className="shrink-0"
              data-testid="thread-term-session-button"
              size="icon"
              type="button"
              variant="ghost"
            >
              <SquareTerminal />
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent>Buzz Term</TooltipContent>
      </Tooltip>
      <PopoverContent
        align="end"
        className="w-80 max-w-[90vw] space-y-3"
        data-testid="thread-term-session-panel"
        onOpenAutoFocus={(event) => event.preventDefault()}
      >
        <div>
          <div className="text-sm font-semibold">
            Pass thread context to Buzz Term
          </div>
          <p className="mt-0.5 text-2xs text-muted-foreground">
            Ask a local channel agent to summarize this thread into a Term
            handoff card.
          </p>
        </div>

        <label className="flex flex-col gap-1 text-sm">
          <span className="text-muted-foreground">Agent</span>
          {agents.length === 0 ? (
            <p
              className="rounded-md border border-dashed border-border px-2 py-1.5 text-sm text-muted-foreground"
              data-testid="thread-term-session-agent-empty"
            >
              {NO_HANDOFF_AGENTS_LABEL}
            </p>
          ) : (
            <select
              className="rounded-md border border-border bg-background px-2 py-1.5"
              data-testid="thread-term-session-agent"
              onChange={(event) => setAgentPubkey(event.target.value)}
              value={agentPubkey}
            >
              <option value="">Select agent…</option>
              {agents.map((agent) => (
                <option key={agent.pubkey} value={agent.pubkey}>
                  {agent.name || agent.pubkey.slice(0, 12)}
                </option>
              ))}
            </select>
          )}
        </label>

        <label className="flex flex-col gap-1 text-sm">
          <span className="text-muted-foreground">Harness</span>
          <select
            className="rounded-md border border-border bg-background px-2 py-1.5"
            data-testid="thread-term-session-harness"
            onChange={(event) =>
              setHarness(event.target.value as TermSessionTool)
            }
            value={harness}
          >
            {harnesses.map((option) => (
              <option
                disabled={!option.available}
                key={option.id}
                value={option.id}
              >
                {option.label}
                {option.available ? "" : " (not installed)"}
              </option>
            ))}
          </select>
          {selectedHarness && !selectedHarness.available ? (
            <span
              className="text-2xs text-muted-foreground"
              data-testid="thread-term-session-harness-hint"
            >
              {selectedHarness.installHint}
            </span>
          ) : null}
        </label>

        <div className="flex justify-end">
          <Button
            data-testid="thread-term-session-go"
            disabled={!canGo}
            onClick={() => void handleGo()}
            size="sm"
            type="button"
          >
            {busy ? "Working…" : "Go"}
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );
}
