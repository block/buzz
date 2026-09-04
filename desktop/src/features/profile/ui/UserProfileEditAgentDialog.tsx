import {
  AgentEditDialog,
  type AgentEditContext,
} from "@/features/agents/ui/AgentEditDialog";
import type { EditAgentFocusTarget } from "@/features/agents/openEditAgentEvent";
import type { AgentPersona, ManagedAgent } from "@/shared/api/types";

export function UserProfileEditAgentDialog({
  canEdit,
  initialFocus,
  isIdentityArchived,
  linkedInstanceCount,
  managedAgent,
  onDeletePersona,
  onOpenChange,
  open,
  resolvedPersona,
}: {
  canEdit: boolean;
  initialFocus: EditAgentFocusTarget | undefined;
  isIdentityArchived: boolean;
  linkedInstanceCount: number;
  managedAgent: ManagedAgent | undefined;
  onDeletePersona: () => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  resolvedPersona: AgentPersona | undefined;
}) {
  const ctx: AgentEditContext | null =
    managedAgent && resolvedPersona
      ? {
          kind: "instance-with-definition",
          definition: resolvedPersona,
          instance: managedAgent,
        }
      : managedAgent
        ? { kind: "instance-only", instance: managedAgent }
        : resolvedPersona
          ? { kind: "definition-only", definition: resolvedPersona }
          : null;

  if (!canEdit || !ctx) {
    return null;
  }

  return (
    <AgentEditDialog
      ctx={ctx}
      initialFocus={initialFocus}
      isIdentityArchived={isIdentityArchived}
      linkedInstanceCount={linkedInstanceCount}
      onDeleteDefinition={
        resolvedPersona?.isBuiltIn ? undefined : onDeletePersona
      }
      onOpenChange={onOpenChange}
      onRemoveFromMyAgents={
        resolvedPersona?.isBuiltIn ? onDeletePersona : undefined
      }
      open={open}
    />
  );
}
