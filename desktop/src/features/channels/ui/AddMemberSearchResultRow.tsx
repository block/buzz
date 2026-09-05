import { AgentManagementMarker } from "@/features/agents/ui/OtherSetupAgentMarker";
import { Bot } from "lucide-react";
import type { UserSearchResult } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { UserAvatar } from "@/shared/ui/UserAvatar";

const MEMBER_ROW_INSET_DIVIDER_CLASS =
  "after:pointer-events-none after:absolute after:bottom-0 after:left-[3.75rem] after:right-0 after:h-px after:bg-border/60 after:content-[''] last:after:hidden";

export function formatAddCandidateName(user: UserSearchResult) {
  return (
    user.displayName?.trim() ||
    user.nip05Handle?.trim() ||
    truncatePubkey(user.pubkey)
  );
}

export function AddMemberSearchResultRow({
  disabled,
  onSelect,
  onSelectWithoutStarting,
  ownerLabel,
  user,
}: {
  disabled: boolean;
  onSelect: (user: UserSearchResult) => void;
  onSelectWithoutStarting?: (user: UserSearchResult) => void;
  ownerLabel?: string | null;
  user: UserSearchResult;
}) {
  const candidateName = formatAddCandidateName(user);

  return (
    <div
      className={cn(
        "group/add-result relative isolate flex min-h-14 w-full items-center gap-3 px-4 py-3.5 text-left transition-colors duration-150 ease-out hover:bg-muted/40 focus-within:bg-muted/40",
        MEMBER_ROW_INSET_DIVIDER_CLASS,
      )}
      data-testid={`channel-user-search-result-${user.pubkey}`}
    >
      <span
        aria-hidden="true"
        className={cn(
          "absolute inset-0 z-0 cursor-pointer",
          disabled && "pointer-events-none cursor-default",
        )}
        onClick={() => {
          if (!disabled) onSelect(user);
        }}
      />
      <UserAvatar
        avatarUrl={user.avatarUrl}
        className="pointer-events-none relative z-10 h-8 w-8 text-xs shadow-none"
        displayName={candidateName}
        shape={user.isAgent ? "squircle" : "circle"}
        size="sm"
      />
      <div className="pointer-events-none relative z-10 min-w-0 flex-1">
        {user.isAgent ? (
          <div className="min-w-0">
            <div className="flex min-w-0 items-center gap-2">
              <span className="truncate text-sm font-medium tracking-tight">
                {candidateName}
              </span>
              <span className="inline-flex shrink-0 items-center gap-1 text-xs text-muted-foreground">
                <Bot aria-hidden="true" className="h-4 w-4" />
                agent
              </span>
              <AgentManagementMarker
                pubkey={user.pubkey}
                ownerPubkey={user.ownerPubkey}
              />
            </div>
            <span className="block truncate font-mono text-2xs text-muted-foreground">
              {truncatePubkey(user.pubkey)}
            </span>
            {ownerLabel ? (
              <span className="block truncate text-xs text-muted-foreground">
                managed by {ownerLabel}
              </span>
            ) : null}
          </div>
        ) : (
          <span className="block truncate text-sm font-medium tracking-tight">
            {candidateName}
          </span>
        )}
      </div>
      <div className="relative z-20 flex shrink-0 items-center gap-2">
        {onSelectWithoutStarting ? (
          <Button
            aria-label={`Add ${candidateName} without starting`}
            disabled={disabled}
            onClick={(event) => {
              event.stopPropagation();
              onSelectWithoutStarting(user);
            }}
            size="sm"
            type="button"
            variant="outline"
          >
            Add without starting
          </Button>
        ) : null}
        <Button
          aria-label={
            onSelectWithoutStarting
              ? `Add ${candidateName} and start`
              : `Add ${candidateName}`
          }
          disabled={disabled}
          onClick={(event) => {
            event.stopPropagation();
            onSelect(user);
          }}
          size="sm"
          type="button"
        >
          {onSelectWithoutStarting ? "Add and start" : "Add"}
        </Button>
      </div>
    </div>
  );
}
