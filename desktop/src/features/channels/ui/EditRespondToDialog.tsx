import * as React from "react";

import { useUpdateManagedAgentMutation } from "@/features/agents/hooks";
import { CreateAgentRespondToField } from "@/features/agents/ui/RespondToField";
import type { ManagedAgent, RespondToMode } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

export function EditRespondToDialog({
  agent,
  currentPubkey,
  onOpenChange,
  open,
}: {
  agent: ManagedAgent | null;
  currentPubkey?: string;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const updateMutation = useUpdateManagedAgentMutation();
  const [respondTo, setRespondTo] = React.useState<RespondToMode>("owner-only");
  const [respondToAllowlist, setRespondToAllowlist] = React.useState<string[]>(
    [],
  );

  React.useEffect(() => {
    if (agent) {
      setRespondTo(agent.respondTo);
      setRespondToAllowlist([...agent.respondToAllowlist]);
    }
  }, [agent]);

  const respondToValid =
    respondTo !== "allowlist" || respondToAllowlist.length > 0;

  async function handleSave() {
    if (!agent) return;
    await updateMutation.mutateAsync({
      pubkey: agent.pubkey,
      respondTo,
      respondToAllowlist:
        respondTo === "allowlist" ? respondToAllowlist : undefined,
    });
    onOpenChange(false);
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Edit respond-to</DialogTitle>
          <DialogDescription>
            Choose who {agent?.name ?? "this agent"} responds to.
          </DialogDescription>
        </DialogHeader>
        <CreateAgentRespondToField
          allowlist={respondToAllowlist}
          disabled={updateMutation.isPending}
          mode={respondTo}
          onAllowlistChange={setRespondToAllowlist}
          onModeChange={setRespondTo}
          ownerPubkey={currentPubkey}
        />
        {updateMutation.error instanceof Error ? (
          <p className="text-sm text-destructive">
            {updateMutation.error.message}
          </p>
        ) : null}
        <div className="flex justify-end gap-2">
          <Button
            onClick={() => onOpenChange(false)}
            size="sm"
            type="button"
            variant="outline"
          >
            Cancel
          </Button>
          <Button
            disabled={!respondToValid || updateMutation.isPending}
            onClick={() => void handleSave()}
            size="sm"
            type="button"
          >
            {updateMutation.isPending ? "Saving..." : "Save"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
