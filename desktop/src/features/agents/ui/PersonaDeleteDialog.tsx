import type { AgentPersona } from "@/shared/api/types";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";

type PersonaDeleteDialogProps = {
  open: boolean;
  persona: AgentPersona | null;
  /** Number of managed-agent instances backed by this persona. Omit or pass 0 to suppress the instance-count sentence. */
  instanceCount?: number;
  /** How many of those instances are provider-hosted. Omit or pass 0 to suppress the provider-removal sentence. */
  providerInstanceCount?: number;
  onConfirm: (persona: AgentPersona) => void;
  onOpenChange: (open: boolean) => void;
};

/**
 * Confirmation copy for deleting a persona. Pure so the cascade archival
 * disclosure stays unit-testable without a renderer: whenever instances are
 * cascade-deleted, each one's identity is also archived on the relay
 * (NIP-IA), and that durable side effect must be disclosed before the
 * destructive confirm — matching the direct agent-delete dialog.
 *
 * Provider-hosted instances are disclosed separately because the cascade does
 * NOT tear them down. `delete_managed_agent` stops the local process, drops the
 * record and key, and tombstones/archives the identity — it never contacts the
 * provider, and `force_remote_delete` only bypasses the backend's orphan guard.
 * Provider teardown is a future `undeploy` operation (see the note in
 * `managed_agents/runtime.rs`), so the remote deployment keeps running and
 * keeps costing money. Saying otherwise would be worse than saying nothing.
 */
export function personaDeleteDescription(
  persona: AgentPersona | null,
  instanceCount: number,
  providerInstanceCount = 0,
): string {
  if (!persona) {
    return "Delete this agent.";
  }
  if (instanceCount === 0) {
    return `Delete ${persona.displayName}.`;
  }
  const cascade =
    instanceCount === 1
      ? "Also deletes 1 agent instance and archives its identity on the relay, so it no longer appears in member lists or mention suggestions."
      : `Also deletes ${instanceCount} agent instances and archives their identities on the relay, so they no longer appear in member lists or mention suggestions.`;
  if (providerInstanceCount === 0) {
    return `Delete ${persona.displayName}. ${cascade}`;
  }
  const provider =
    providerInstanceCount === 1
      ? "1 of them is hosted by a provider: the remote deployment is not torn down and keeps running until you remove it at the provider."
      : `${providerInstanceCount} of them are hosted by a provider: those remote deployments are not torn down and keep running until you remove them at the provider.`;
  return `Delete ${persona.displayName}. ${cascade} ${provider}`;
}

export function PersonaDeleteDialog({
  open,
  persona,
  instanceCount = 0,
  providerInstanceCount = 0,
  onConfirm,
  onOpenChange,
}: PersonaDeleteDialogProps) {
  return (
    <AlertDialog onOpenChange={onOpenChange} open={open}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete agent?</AlertDialogTitle>
          <AlertDialogDescription>
            {personaDeleteDescription(
              persona,
              instanceCount,
              providerInstanceCount,
            )}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button type="button" variant="outline">
              Cancel
            </Button>
          </AlertDialogCancel>
          <AlertDialogAction asChild>
            <Button
              onClick={() => {
                if (persona) {
                  onConfirm(persona);
                }
              }}
              type="button"
              variant="destructive"
            >
              Delete
            </Button>
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
