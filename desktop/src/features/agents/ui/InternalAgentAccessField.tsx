import type { RespondToMode } from "@/shared/api/types";
import {
  CreateAgentRespondToField,
  INTERNAL_AGENT_ACCESS_DISABLED_REASON,
} from "./RespondToField";

export function InternalAgentAccessField({
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
        accessLocked ? INTERNAL_AGENT_ACCESS_DISABLED_REASON : undefined
      }
      mode={accessLocked ? "owner-only" : mode}
      onAllowlistChange={onAllowlistChange}
      onModeChange={onModeChange}
      variant="persona"
    />
  );
}
