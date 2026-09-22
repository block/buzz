import { toast } from "sonner";

import { useTranslation } from "@/i18n";

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
  const { t } = useTranslation();
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
          <DialogTitle>
            {t("members.remove.title", { name: label })}
          </DialogTitle>
          <DialogDescription>
            {t("members.remove.description")}
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
            {t("members.remove.cancel")}
          </Button>
          <Button
            data-testid="confirm-remove-member"
            disabled={removeMutation.isPending || !member}
            onClick={() => {
              if (!member) return;
              removeMutation.mutate(member.pubkey, {
                onSuccess: () => {
                  toast.success(t("members.remove.removed"));
                  handleOpenChange(false);
                },
                onError: (error) => {
                  toast.error(
                    error instanceof Error
                      ? error.message
                      : t("members.remove.failed"),
                  );
                },
              });
            }}
            size="sm"
            variant="destructive"
          >
            {removeMutation.isPending
              ? t("members.remove.removing")
              : t("members.remove.confirm")}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
