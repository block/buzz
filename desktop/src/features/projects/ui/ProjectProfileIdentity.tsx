import type * as React from "react";

import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { cn } from "@/shared/lib/cn";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type ProfileIdentityButtonProps = {
  align?: "center" | "start";
  avatarClassName?: string;
  avatarSize?: "xs" | "sm" | "md";
  avatarUrl: string | null;
  isAgent: boolean;
  label: string;
  pubkey: string | null;
  role?: React.ReactNode;
  showLabel?: boolean;
  textSize?: "xs" | "sm";
};

export function ProfileIdentityButton({
  avatarClassName,
  avatarSize = "xs",
  avatarUrl,
  align = "start",
  isAgent,
  label,
  pubkey,
  role,
  showLabel = true,
  textSize = "xs",
}: ProfileIdentityButtonProps) {
  const labelClassName = textSize === "sm" ? "text-sm leading-5" : "text-xs";
  const roleClassName = textSize === "sm" ? "text-sm leading-5" : "text-xs";
  const className = cn(
    "flex min-w-0 rounded-none text-left",
    align === "center" ? "items-center" : "items-start",
    pubkey &&
      "focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
    showLabel ? "gap-2" : "shrink-0",
  );
  const inner = (
    <>
      <UserAvatar
        accent={isAgent}
        avatarUrl={avatarUrl}
        className={avatarClassName}
        displayName={label}
        size={avatarSize}
      />
      {showLabel ? (
        <span className="min-w-0">
          <span
            className={cn(
              "block truncate font-medium text-foreground",
              labelClassName,
              pubkey && "hover:underline",
            )}
          >
            {label}
          </span>
          {role ? (
            <span
              className={cn(
                "block truncate text-muted-foreground",
                roleClassName,
              )}
            >
              {role}
            </span>
          ) : null}
        </span>
      ) : null}
    </>
  );

  if (!pubkey) {
    return <div className={className}>{inner}</div>;
  }

  return (
    <UserProfilePopover pubkey={pubkey} triggerElement="span">
      <button className={className} type="button">
        {inner}
      </button>
    </UserProfilePopover>
  );
}

export function ProfileAuthorName({
  children,
  pubkey,
}: {
  children: React.ReactNode;
  pubkey: string | null;
}) {
  if (!pubkey) {
    return <span className="font-semibold text-foreground">{children}</span>;
  }

  return (
    <UserProfilePopover pubkey={pubkey} triggerElement="span">
      <button
        className="rounded-md font-semibold text-foreground hover:underline focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
        type="button"
      >
        {children}
      </button>
    </UserProfilePopover>
  );
}
