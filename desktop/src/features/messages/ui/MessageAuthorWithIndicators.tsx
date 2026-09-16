import type * as React from "react";

import { MessageAuthorIdentity } from "@/features/messages/ui/MessageHeader";
import { UserNameIndicators } from "@/features/user-status/ui/UserNameIndicators";

type MessageAuthorWithIndicatorsProps = {
  authorName: string;
  children: React.ReactNode;
  ownerPubkey?: string | null;
  pubkey: string;
  role?: string;
};

export function MessageAuthorWithIndicators({
  authorName,
  children,
  ownerPubkey,
  pubkey,
  role,
}: MessageAuthorWithIndicatorsProps) {
  return (
    <span className="inline-flex min-w-0 items-baseline gap-1">
      <MessageAuthorIdentity
        displayName={authorName}
        ownerPubkey={ownerPubkey}
        pubkey={pubkey}
        role={role}
      >
        {children}
      </MessageAuthorIdentity>
      <UserNameIndicators pubkey={pubkey} />
    </span>
  );
}
