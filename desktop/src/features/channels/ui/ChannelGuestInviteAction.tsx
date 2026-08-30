import { UserPlus } from "lucide-react";
import * as React from "react";

import { useMyRelayMembershipLookupQuery } from "@/features/community-members/hooks";
import {
  DEFAULT_INVITE_TTL_SECS,
  InviteLinkSection,
} from "@/features/community-members/ui/InviteLinkSection";
import type { Channel } from "@/shared/api/types";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { ActionFieldRow } from "./ChannelManagementSheetRows";

export function ChannelGuestInviteAction({
  canEditChannel,
  channel,
}: {
  canEditChannel: boolean;
  channel: Channel;
}) {
  const [open, setOpen] = React.useState(false);
  const [ttlSecs, setTtlSecs] = React.useState(DEFAULT_INVITE_TTL_SECS);
  const relayRole = useMyRelayMembershipLookupQuery().data?.membership?.role;
  const canInvite =
    canEditChannel && (relayRole === "owner" || relayRole === "admin");

  if (!canInvite || channel.visibility !== "private" || channel.archivedAt) {
    return null;
  }

  return (
    <>
      <ActionFieldRow
        description="Create a single-use link for this channel only"
        icon={UserPlus}
        label="Invite guest"
        onClick={() => setOpen(true)}
        testId="channel-management-invite-guest"
      />
      <Dialog onOpenChange={setOpen} open={open}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Invite a guest to #{channel.name}</DialogTitle>
            <DialogDescription>
              Guests can read and post in this channel. They cannot browse other
              channels, upload file attachments, direct-message people, or use
              community tools. Existing file links may remain readable when your
              relay serves media publicly.
            </DialogDescription>
          </DialogHeader>
          <InviteLinkSection
            channelId={channel.id}
            onTtlSecsChange={setTtlSecs}
            ttlSecs={ttlSecs}
          />
        </DialogContent>
      </Dialog>
    </>
  );
}
