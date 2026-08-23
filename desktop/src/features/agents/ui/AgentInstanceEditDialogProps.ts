import type { EditAgentFocusTarget } from "@/features/agents/openEditAgentEvent";
import type { ManagedAgent } from "@/shared/api/types";

export type AgentInstanceEditDialogProps = {
  agent: ManagedAgent;
  initialFocus?: EditAgentFocusTarget;
  open: boolean;
  onEditLinkedPersona?: () => void;
  onOpenChange: (open: boolean) => void;
  onUpdated?: (agent: ManagedAgent) => void;
};
