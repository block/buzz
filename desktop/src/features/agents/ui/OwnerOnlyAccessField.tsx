import React from "react";
import type { ManagedAgent, RespondToMode } from "@/shared/api/types";
import {
  CreateAgentRespondToField,
  OWNER_ONLY_ACCESS_DISABLED_REASON,
} from "./RespondToField";

/**
 * Manages respondTo/respondToAllowlist state for the edit dialog.
 * Returns the values, setters, and a `reset` function for discard/re-open.
 */
export function useRespondToField(
  agent: Pick<ManagedAgent, "respondTo" | "respondToAllowlist">,
) {
  const [respondTo, setRespondTo] = React.useState<RespondToMode>(
    agent.respondTo,
  );
  const [respondToAllowlist, setRespondToAllowlist] = React.useState<string[]>(
    agent.respondToAllowlist,
  );
  const reset = React.useCallback(() => {
    setRespondTo(agent.respondTo);
    setRespondToAllowlist(agent.respondToAllowlist);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agent.respondTo, agent.respondToAllowlist]);
  return {
    respondTo,
    setRespondTo,
    respondToAllowlist,
    setRespondToAllowlist,
    reset,
  };
}

export function OwnerOnlyAccessField({
  accessLocked,
  allowlist,
  disabled,
  mode,
  onAllowlistChange,
  onModeChange,
}: {
  accessLocked: boolean;
  allowlist: string[];
  disabled: boolean;
  mode: RespondToMode;
  onAllowlistChange: (allowlist: string[]) => void;
  onModeChange: (mode: RespondToMode) => void;
}) {
  return (
    <CreateAgentRespondToField
      allowlist={accessLocked ? [] : allowlist}
      disabled={disabled || accessLocked}
      disabledReason={
        accessLocked ? OWNER_ONLY_ACCESS_DISABLED_REASON : undefined
      }
      mode={accessLocked ? "owner-only" : mode}
      onAllowlistChange={onAllowlistChange}
      onModeChange={onModeChange}
      variant="persona"
    />
  );
}
