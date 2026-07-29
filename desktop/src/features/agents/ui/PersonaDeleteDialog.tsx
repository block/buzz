import type { PersonaRemoteCascadeInstance } from "@/features/agents/lib/personaCascade";
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
  /**
   * Cascade instances with a live provider deployment. Non-empty means the
   * delete leaves remote units running, which the dialog must say before the
   * destructive confirm.
   */
  remoteInstances?: readonly PersonaRemoteCascadeInstance[];
  onConfirm: (persona: AgentPersona) => void;
  onOpenChange: (open: boolean) => void;
};

/**
 * Confirmation copy for deleting a persona. Pure so the cascade archival
 * disclosure stays unit-testable without a renderer: whenever instances are
 * cascade-deleted, each one's identity is also archived on the relay
 * (NIP-IA), and that durable side effect must be disclosed before the
 * destructive confirm — matching the direct agent-delete dialog.
 */
export function personaDeleteDescription(
  persona: AgentPersona | null,
  instanceCount: number,
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
  return `Delete ${persona.displayName}. ${cascade}`;
}

/**
 * Disclosure for cascade instances whose remote deployment survives the delete.
 *
 * The provider protocol is deploy-only — there is no undeploy — so this app can
 * remove the record and nothing more. Naming each unit is the difference
 * between a warning the owner can act on and one they can only worry about, so
 * the copy carries `backend_agent_id` verbatim.
 *
 * Returns `null` when the cascade orphans nothing, so the caller renders no
 * warning at all rather than a hedged one.
 */
export function personaDeleteRemoteWarning(
  remoteInstances: readonly PersonaRemoteCascadeInstance[],
): string | null {
  if (remoteInstances.length === 0) {
    return null;
  }
  const units = remoteInstances
    .map((instance) => `${instance.name} (${instance.unitId})`)
    .join(", ");
  const subject =
    remoteInstances.length === 1
      ? "1 of these instances is deployed remotely"
      : `${remoteInstances.length} of these instances are deployed remotely`;
  return (
    `${subject}: ${units}. Deleting removes them from this app, but does not ` +
    `stop them — Buzz cannot tear down a remote deployment. They keep running ` +
    `until stopped on the host.`
  );
}

export function PersonaDeleteDialog({
  open,
  persona,
  instanceCount = 0,
  remoteInstances = [],
  onConfirm,
  onOpenChange,
}: PersonaDeleteDialogProps) {
  const remoteWarning = personaDeleteRemoteWarning(remoteInstances);
  return (
    <AlertDialog onOpenChange={onOpenChange} open={open}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete agent?</AlertDialogTitle>
          <AlertDialogDescription>
            {personaDeleteDescription(persona, instanceCount)}
          </AlertDialogDescription>
        </AlertDialogHeader>
        {/* Deliberately a plain <p> outside the header, not a second
            AlertDialogDescription: Radix gives every Description in one Content
            the same generated id, so aria-describedby would resolve to the
            first one and a screen reader would never reach this warning — the
            one sentence on this dialog that reports an irreversible remote
            side effect. Mirrors AgentDeleteConfirmDialog. */}
        {remoteWarning ? (
          <p
            className="text-sm text-destructive"
            data-testid="persona-delete-remote-warning"
          >
            {remoteWarning}
          </p>
        ) : null}
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
