import * as React from "react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { DirectAddMemberForm } from "./AddMemberDialog";
import {
  DEFAULT_INVITE_TTL_SECS,
  InviteLinkSection,
} from "./InviteLinkSection";
import { Separator } from "@/shared/ui/separator";

export function CommunityInviteDialog({
  isOwner,
  onOpenChange,
  open,
}: {
  isOwner: boolean;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const [ttlSecs, setTtlSecs] = React.useState(DEFAULT_INVITE_TTL_SECS);

  React.useEffect(() => {
    // Reset after the link section has unmounted so reopening never mints an
    // invite with the previous dialog session's expiry.
    if (!open) setTtlSecs(DEFAULT_INVITE_TTL_SECS);
  }, [open]);

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        className="max-h-[85vh] max-w-xl overflow-y-auto"
        data-testid="community-invite-dialog"
      >
        <DialogHeader>
          <DialogTitle>Invite to community</DialogTitle>
          <DialogDescription>
            Add someone directly or share a link they can use to join.
          </DialogDescription>
        </DialogHeader>

        <section className="mt-2 space-y-3">
          <DirectAddMemberForm
            isOwner={isOwner}
            showLabel={false}
            submitLabel="Invite"
          />
        </section>

        <div
          className="relative flex items-center py-2"
          data-testid="invite-options-divider"
        >
          <Separator className="bg-input/40" />
          <span className="absolute left-1/2 -translate-x-1/2 bg-background px-3 text-sm text-muted-foreground">
            Or, copy a link
          </span>
        </div>

        <section className="space-y-3">
          <InviteLinkSection onTtlSecsChange={setTtlSecs} ttlSecs={ttlSecs} />
        </section>
      </DialogContent>
    </Dialog>
  );
}
