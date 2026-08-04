import * as React from "react";

import { useCommunities } from "@/features/communities/useCommunities";
import { useProjectsQuery } from "@/features/projects/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { durableProjectAddress } from "@/shared/api/agentProjectTypes";
import type { AgentPersona } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import type { AgentLaunchContext } from "./agentCreateIntent";
import {
  AgentProjectAccessSection,
  emptyAgentProjectAccessDraft,
  type AgentProjectAccessReadiness,
} from "./AgentProjectAccessSection";

export function AgentProjectLaunchDialog({
  error,
  isPending,
  onOpenChange,
  onStart,
  open,
  persona,
}: {
  error: string | null;
  isPending: boolean;
  onOpenChange: (open: boolean) => void;
  onStart: (context: AgentLaunchContext) => Promise<boolean>;
  open: boolean;
  persona: AgentPersona;
}) {
  const projectsQuery = useProjectsQuery();
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  const [draft, setDraft] = React.useState(emptyAgentProjectAccessDraft);
  const [readiness, setReadiness] = React.useState<AgentProjectAccessReadiness>(
    {
      ready: false,
      reason: "Choose a Project for this agent.",
    },
  );

  React.useEffect(() => {
    if (!open) return;
    setDraft(emptyAgentProjectAccessDraft);
    setReadiness({
      ready: false,
      reason: "Choose a Project for this agent.",
    });
  }, [open]);

  const handleReadinessChange = React.useCallback(
    (next: AgentProjectAccessReadiness) => {
      setReadiness((current) =>
        current.ready === next.ready && current.reason === next.reason
          ? current
          : next,
      );
    },
    [],
  );

  async function handleStart() {
    const project = (projectsQuery.data ?? []).find(
      (candidate) => candidate.id === draft.projectId,
    );
    if (
      !readiness.ready ||
      !project?.projectChannelId ||
      !activeCommunity?.relayUrl ||
      !identityQuery.data?.pubkey
    ) {
      return;
    }

    const started = await onStart({
      projectScope: {
        relayUrl: activeCommunity.relayUrl,
        operatorPubkey: identityQuery.data.pubkey,
        projectAddress: durableProjectAddress(project),
        channelId: project.projectChannelId,
      },
      connectionBindings: draft.connectionBindings,
    });
    if (started) onOpenChange(false);
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        className="max-h-[calc(100vh-2rem)] max-w-xl overflow-y-auto"
        data-testid="agent-project-launch-dialog"
      >
        <DialogHeader>
          <DialogTitle>Start {persona.displayName}</DialogTitle>
          <DialogDescription>
            Choose the Project this agent will work in and connect the tools it
            needs.
          </DialogDescription>
        </DialogHeader>

        <AgentProjectAccessSection
          disabled={isPending}
          draft={draft}
          onDraftChange={setDraft}
          onReadinessChange={handleReadinessChange}
          operatorPubkey={identityQuery.data?.pubkey ?? null}
          projects={projectsQuery.data ?? []}
          projectsLoading={projectsQuery.isPending}
          relayUrl={activeCommunity?.relayUrl ?? null}
          toolRequirements={persona.toolRequirements ?? []}
        />

        {error ? (
          <p className="text-sm text-destructive" role="alert">
            {error}
          </p>
        ) : !readiness.ready && readiness.reason ? (
          <p className="text-xs text-muted-foreground" aria-live="polite">
            {readiness.reason}
          </p>
        ) : null}

        <div className="flex items-center justify-end gap-2">
          <DialogClose asChild>
            <Button disabled={isPending} type="button" variant="outline">
              Cancel
            </Button>
          </DialogClose>
          <Button
            data-testid="agent-project-launch-submit"
            disabled={isPending || !readiness.ready}
            onClick={() => void handleStart()}
            type="button"
          >
            {isPending ? "Starting..." : "Start agent"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
