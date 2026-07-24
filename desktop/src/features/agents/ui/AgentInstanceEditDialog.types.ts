import type { EditAgentFocusTarget } from "@/features/agents/openEditAgentEvent";
import type { ManagedAgent } from "@/shared/api/types";

export type AgentInstanceEditDialogProps = {
  agent: ManagedAgent;
  /** Optional field to scroll/focus when the dialog opens from a card deep-link. */
  initialFocus?: EditAgentFocusTarget;
  open: boolean;
  /** Present only when the linked definition is editable (non-built-in,
   * resolved). Caller closes this dialog and enters definition-edit. */
  onEditLinkedPersona?: () => void;
  onOpenChange: (open: boolean) => void;
  onUpdated?: (agent: ManagedAgent) => void;
};
