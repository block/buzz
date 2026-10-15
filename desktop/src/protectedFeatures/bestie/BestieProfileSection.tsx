import * as React from "react";
import { Star } from "lucide-react";
import { toast } from "sonner";

import { ProfileAgentActionRow } from "@/features/profile/ui/UserProfileAgentManagementRows";
import type { ManagedAgent } from "@/shared/api/types";
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
import { useBestie } from "./useBestie";

export function BestieProfileAction({ agent }: { agent: ManagedAgent }) {
  const bestie = useBestie();
  const [confirmOpen, setConfirmOpen] = React.useState(false);
  if (agent.backend.type !== "local") return null;

  const isBestie =
    bestie.assignment?.agentPubkey.toLowerCase() === agent.pubkey.toLowerCase();
  const isPending = bestie.isAssigning;
  const handleClick = () => {
    if (!isBestie) {
      setConfirmOpen(true);
      return;
    }
    void bestie.clearAssignment().catch((error) => {
      toast.error(
        error instanceof Error ? error.message : "Couldn’t update Bestie",
      );
    });
  };

  return (
    <>
      <ProfileAgentActionRow
        disabled={bestie.isLoading || isPending}
        icon={Star}
        iconClassName={
          isBestie ? "h-4 w-4 shrink-0 fill-current text-foreground" : undefined
        }
        label={isBestie ? "Remove Bestie" : "Make Bestie"}
        onClick={handleClick}
        testId="user-profile-bestie-action"
      />
      <AlertDialog onOpenChange={setConfirmOpen} open={confirmOpen}>
        <AlertDialogContent data-testid="bestie-confirm-dialog">
          <AlertDialogHeader>
            <AlertDialogTitle>Make {agent.name} your Bestie?</AlertDialogTitle>
            <AlertDialogDescription>
              {bestie.assignedAgent && !isBestie
                ? `${agent.name} will replace ${bestie.assignedAgent.name} in the floating shortcut and message actions.`
                : `${agent.name} will appear in the floating shortcut and message actions.`}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              disabled={isPending}
              onClick={(event) => {
                event.preventDefault();
                void bestie
                  .assignAgent(agent)
                  .then(() => setConfirmOpen(false))
                  .catch((error) => {
                    toast.error(
                      error instanceof Error
                        ? error.message
                        : "Couldn’t update Bestie",
                    );
                  });
              }}
            >
              {isPending ? "Saving…" : "Make Bestie"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
