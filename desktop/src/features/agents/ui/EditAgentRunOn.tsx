import * as React from "react";

import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import type {
  ManagedAgent,
  ManagedAgentBackend,
  RespondToMode,
} from "@/shared/api/types";
import {
  runLocationForBackend,
  runLocationForRunOn,
} from "../lib/agentAccessWarning";
import { AgentRunLocationProvider } from "./AgentRunLocationContext";
import { OwnerOnlyAccessField } from "./OwnerOnlyAccessField";
import { RunOnSummarySection } from "./RunOnSummarySection";
import { WhereToRunSection } from "./WhereToRunSection";
import {
  canSubmitWhereToRun,
  emptyWhereToRunDraft,
  resolveBackendIntent,
} from "./whereToRunIntent";

export type EditAgentRunOnState = {
  backend: ManagedAgentBackend | undefined;
  valid: boolean;
};
export const INITIAL_RUN_ON: EditAgentRunOnState = {
  backend: undefined,
  valid: true,
};

export function EditAgentRunOn({
  accessLocked,
  agent,
  allowlist,
  disabled,
  mode,
  onAllowlistChange,
  onModeChange,
  onRunOnStateChange,
}: {
  accessLocked: boolean;
  agent: ManagedAgent;
  allowlist: string[];
  disabled: boolean;
  mode: RespondToMode;
  onAllowlistChange: (allowlist: string[]) => void;
  onModeChange: (mode: RespondToMode) => void;
  onRunOnStateChange: (state: EditAgentRunOnState) => void;
}) {
  const [runDraft, setRunDraft] = React.useState(emptyWhereToRunDraft);
  const canMigrateBackend =
    agent.backend.type === "local" && !isManagedAgentActive(agent);
  const backendIntent = React.useMemo(
    () => (canMigrateBackend ? resolveBackendIntent(runDraft) : null),
    [canMigrateBackend, runDraft],
  );
  const migrationRequested = backendIntent != null;
  const valid = !canMigrateBackend || canSubmitWhereToRun(runDraft);

  React.useEffect(() => {
    onRunOnStateChange({ backend: backendIntent ?? undefined, valid });
  }, [backendIntent, onRunOnStateChange, valid]);

  return (
    <AgentRunLocationProvider
      runLocation={
        canMigrateBackend
          ? runLocationForRunOn(runDraft.runOn)
          : runLocationForBackend(agent.backend)
      }
    >
      <OwnerOnlyAccessField
        accessLocked={accessLocked}
        allowlist={allowlist}
        disabled={disabled}
        mode={mode}
        onAllowlistChange={onAllowlistChange}
        onModeChange={onModeChange}
      />
      {canMigrateBackend ? (
        <div className="space-y-2" data-testid="edit-agent-run-on-migration">
          <WhereToRunSection
            draft={runDraft}
            isPending={disabled}
            onDraftChange={setRunDraft}
          />
          {migrationRequested ? (
            <p className="text-xs text-muted-foreground">
              Saving preserves this agent&apos;s identity and settings, but
              leaves it stopped until you explicitly deploy it.
            </p>
          ) : null}
        </div>
      ) : (
        <RunOnSummarySection
          backend={agent.backend}
          migrationBlockedReason={
            agent.backend.type === "local"
              ? "Stop this agent before changing where it runs."
              : undefined
          }
        />
      )}
    </AgentRunLocationProvider>
  );
}
