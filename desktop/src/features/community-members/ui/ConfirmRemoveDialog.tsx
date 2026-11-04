// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT, useUiLanguage } from "@/shared/i18n/uiTranslation";
import { toast } from "sonner";

import { truncateNpub } from "@/shared/lib/pubkey";
import { PubKey } from "@/shared/ui/PubKey";
import { useRemoveRelayMemberMutation } from "@/features/community-members/hooks";
import type { RelayMember } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

export function ConfirmRemoveDialog({
  member,
  displayName,
  open,
  onOpenChange,
}: {
  member: RelayMember | null;
  displayName: string | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  useUiLanguage();

  const removeMutation = useRemoveRelayMemberMutation();
  const label = displayName || (member ? truncateNpub(member.pubkey) : "");

  function handleOpenChange(next: boolean) {
    if (!next) {
      removeMutation.reset();
    }
    onOpenChange(next);
  }

  return (
    <Dialog onOpenChange={handleOpenChange} open={open}>
      <DialogContent
        className="max-w-sm"
        data-testid="confirm-remove-member-dialog"
      >
        <DialogHeader>
          <DialogTitle>{uiT("Remove {0}?", { p0: label })}</DialogTitle>
          <DialogDescription>
            {uiT("This will immediately revoke their access to the relay.")}
          </DialogDescription>
          {member ? (
            <PubKey
              pubkey={member.pubkey}
              testId="confirm-remove-member-pubkey"
              variant="full"
            />
          ) : null}
        </DialogHeader>
        <div className="flex justify-end gap-2">
          <Button
            onClick={() => handleOpenChange(false)}
            size="sm"
            variant="outline"
          >
            {uiT("Cancel")}
          </Button>
          <Button
            data-testid="confirm-remove-member"
            disabled={removeMutation.isPending || !member}
            onClick={() => {
              if (!member) return;
              removeMutation.mutate(member.pubkey, {
                onSuccess: () => {
                  toast.success(uiT("Member removed"));
                  handleOpenChange(false);
                },
                onError: (error) => {
                  toast.error(
                    error instanceof Error
                      ? error.message
                      : uiT("Failed to remove member"),
                  );
                },
              });
            }}
            size="sm"
            variant="destructive"
          >
            {removeMutation.isPending ? uiT("Removing...") : uiT("Remove")}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
