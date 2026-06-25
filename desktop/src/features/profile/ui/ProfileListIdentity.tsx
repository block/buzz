import { Bot } from "lucide-react";

import { truncatePubkey } from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";

type ProfileListIdentityProps = {
  agentIconClassName?: string;
  agentIconTestId?: string;
  hoverGroup?: "member" | "dm-result";
  isAgent?: boolean;
  label: string;
  ownerLabel?: string | null;
  pubkey: string;
  roleLabel?: string | null;
};

function hoverClasses(hoverGroup: "member" | "dm-result", visible: boolean) {
  const prefix =
    hoverGroup === "dm-result" ? "group-hover/dm-result" : "group-hover/member";
  const focusPrefix =
    hoverGroup === "dm-result"
      ? "group-focus-within/dm-result"
      : "group-focus-within/member";

  return visible
    ? cn(
        "opacity-0 transition-opacity duration-150 ease-out",
        `${prefix}:opacity-100`,
        `${focusPrefix}:opacity-100`,
      )
    : cn(
        "transition-opacity duration-150 ease-out",
        `${prefix}:opacity-0`,
        `${focusPrefix}:opacity-0`,
      );
}

export function ProfileListIdentity({
  agentIconClassName = "h-3 w-3",
  agentIconTestId,
  hoverGroup = "member",
  isAgent = false,
  label,
  ownerLabel,
  pubkey,
  roleLabel = "agent",
}: ProfileListIdentityProps) {
  if (!isAgent) {
    return (
      <div className="flex min-w-0 items-center gap-2">
        <span className="truncate text-sm font-medium tracking-tight">
          {label}
        </span>
        {roleLabel ? (
          <span className="inline-flex shrink-0 items-center text-xs text-muted-foreground">
            {roleLabel}
          </span>
        ) : null}
      </div>
    );
  }

  return (
    <div className="relative min-w-0">
      <div
        className={cn(
          "flex min-w-0 items-center gap-2",
          hoverClasses(hoverGroup, false),
        )}
      >
        <span className="truncate text-sm font-medium tracking-tight">
          {label}
        </span>
        <span className="inline-flex shrink-0 items-center gap-1 text-xs text-muted-foreground">
          <Bot
            aria-hidden="true"
            className={agentIconClassName}
            data-testid={agentIconTestId}
          />
          {roleLabel}
        </span>
      </div>
      {ownerLabel ? (
        <span
          className={cn(
            "block truncate text-xs text-muted-foreground",
            hoverClasses(hoverGroup, false),
          )}
        >
          owned by {ownerLabel}
        </span>
      ) : null}
      <span
        className={cn(
          "absolute inset-0 flex items-center",
          hoverClasses(hoverGroup, true),
        )}
      >
        <span className="truncate font-mono text-sm text-muted-foreground">
          {truncatePubkey(pubkey)}
        </span>
      </span>
    </div>
  );
}
