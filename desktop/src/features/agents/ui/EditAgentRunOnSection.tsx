import * as React from "react";

import { Button } from "@/shared/ui/button";
import type { ManagedAgent, UpdateManagedAgentInput } from "@/shared/api/types";

import { RunOnSummarySection } from "./RunOnSummarySection";
import { WhereToRunSection } from "./WhereToRunSection";
import {
  draftFromBackend,
  resolveBackendEdit,
  type WhereToRunDraft,
} from "./whereToRunIntent";

export const BACKEND_MOVE_CONFIRMATION =
  "This agent was deployed with its current provider and may still be running there. Buzz won't stop or remove that deployment. Change where it runs anyway?";

/** Where-to-run edit state for the edit-agent dialog; resets when it opens. */
export function useRunOnEdit(
  agent: Pick<ManagedAgent, "backend" | "backendAgentId">,
  open: boolean,
) {
  const [draft, setDraft] = React.useState<WhereToRunDraft | null>(null);
  React.useEffect(() => {
    if (open) setDraft(null);
  }, [open]);
  const edit = resolveBackendEdit(agent, draft);
  const backend = edit.backend ?? agent.backend;
  const submission: Pick<
    UpdateManagedAgentInput,
    "backend" | "forceBackendChange"
  > = edit.backend
    ? { backend: edit.backend, forceBackendChange: edit.needsConfirmation }
    : {};
  return {
    agentBackend: agent.backend,
    allowsEffort: backend.type === "local",
    backend,
    confirm: () =>
      !edit.needsConfirmation || window.confirm(BACKEND_MOVE_CONFIRMATION),
    draft,
    setDraft,
    submission,
    valid: edit.valid,
  };
}

/**
 * "Run on" for the edit-agent dialog: the saved summary until the user asks
 * to change it, so opening the dialog never probes a provider binary.
 */
export function EditAgentRunOnSection({
  isPending,
  runOn,
}: {
  isPending: boolean;
  runOn: ReturnType<typeof useRunOnEdit>;
}) {
  if (!runOn.draft) {
    return (
      <div className="space-y-2">
        <RunOnSummarySection backend={runOn.agentBackend} />
        <Button
          aria-label="Change where this agent runs"
          data-testid="edit-agent-run-on-change"
          disabled={isPending}
          onClick={() => runOn.setDraft(draftFromBackend(runOn.agentBackend))}
          size="sm"
          type="button"
          variant="outline"
        >
          Change
        </Button>
      </div>
    );
  }
  return (
    <div className="space-y-2" data-testid="edit-agent-run-on-editor">
      <WhereToRunSection
        draft={runOn.draft}
        isPending={isPending}
        onDraftChange={runOn.setDraft}
      />
      <Button
        data-testid="edit-agent-run-on-keep"
        disabled={isPending}
        onClick={() => runOn.setDraft(null)}
        size="sm"
        type="button"
        variant="ghost"
      >
        Keep current location
      </Button>
    </div>
  );
}
