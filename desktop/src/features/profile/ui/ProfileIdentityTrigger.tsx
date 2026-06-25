import type * as React from "react";

import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { cn } from "@/shared/lib/cn";

export const PROFILE_IDENTITY_FOCUS_CLASS =
  "focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring";

type ProfileIdentityTriggerProps = {
  authorRole?: string;
  botIdenticonValue?: string;
  buttonClassName?: string;
  children: React.ReactNode;
  pubkey: string;
  triggerElement?: "div" | "span";
};

export function ProfileIdentityTrigger({
  authorRole,
  botIdenticonValue,
  buttonClassName,
  children,
  pubkey,
  triggerElement = "div",
}: ProfileIdentityTriggerProps) {
  return (
    <UserProfilePopover
      botIdenticonValue={botIdenticonValue}
      pubkey={pubkey}
      role={authorRole}
      triggerElement={triggerElement}
    >
      <button
        className={cn(PROFILE_IDENTITY_FOCUS_CLASS, buttonClassName)}
        type="button"
      >
        {children}
      </button>
    </UserProfilePopover>
  );
}
