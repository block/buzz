import * as React from "react";

import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  DEFAULT_INVITE_TTL_SECS,
  InviteLinkSection,
} from "./InviteLinkSection";

export function CommunityInviteDialog({
  onOpenChange,
  open,
}: {
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  // Email delivery is not available yet, so the modal only mints shareable
  // invite links through the relay's existing invite flow.
  const [ttlSecs, setTtlSecs] = React.useState(DEFAULT_INVITE_TTL_SECS);

  React.useEffect(() => {
    if (open) setTtlSecs(DEFAULT_INVITE_TTL_SECS);
  }, [open]);

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        aria-describedby={undefined}
        className="max-h-[85vh] max-w-xl overflow-y-auto"
        data-testid="community-invite-dialog"
      >
        <DialogHeader className="space-y-0">
          <DialogTitle>Invite to community</DialogTitle>
        </DialogHeader>

        <InviteLinkSection onTtlSecsChange={setTtlSecs} ttlSecs={ttlSecs} />
      </DialogContent>
    </Dialog>
  );
}
