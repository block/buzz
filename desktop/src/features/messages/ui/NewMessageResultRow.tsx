import { Bot } from "lucide-react";

import { formatOwnerLabel } from "@/features/profile/lib/identity";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { UserSearchResult } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { safeNpub } from "@/shared/lib/nostrUtils";
import { truncatePubkey } from "@/shared/lib/pubkey";

import { formatRecipientName } from "./useNewMessageRecipients";

const RESULT_ROW_INSET_DIVIDER_CLASS =
  "after:pointer-events-none after:absolute after:bottom-0 after:left-[3.75rem] after:right-0 after:h-px after:bg-border/60 after:content-[''] last:after:hidden";

/**
 * A single selectable person/agent row in the new-message directory. Extracted
 * from the former NewDirectMessageDialog so the compose page renders identical
 * rows (avatar, agent badge, owner label, hover-revealed npub).
 */
export function NewMessageResultRow({
  currentPubkey,
  disabled,
  isKeyboardHighlighted = false,
  onSelect,
  ownerProfiles,
  user,
}: {
  currentPubkey?: string;
  disabled: boolean;
  isKeyboardHighlighted?: boolean;
  onSelect: (user: UserSearchResult) => void;
  ownerProfiles?: UserProfileLookup;
  user: UserSearchResult;
}) {
  const name = formatRecipientName(user);
  const ownerLabel = formatOwnerLabel(
    user.ownerPubkey,
    currentPubkey,
    ownerProfiles,
  );

  return (
    <div
      className={cn(
        "group/dm-result relative flex min-h-14 w-full items-center gap-3 px-4 py-3.5 text-left transition-colors duration-150 ease-out hover:bg-muted/40 focus-within:bg-muted/40",
        isKeyboardHighlighted && "bg-muted/40",
        RESULT_ROW_INSET_DIVIDER_CLASS,
      )}
      data-keyboard-highlighted={isKeyboardHighlighted ? "true" : undefined}
    >
      <button
        aria-label={`Add ${name}`}
        aria-selected={isKeyboardHighlighted}
        className="absolute inset-0 z-0 cursor-pointer focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
        data-testid={`new-dm-result-${user.pubkey}`}
        disabled={disabled}
        id={`new-dm-option-${user.pubkey}`}
        onClick={() => onSelect(user)}
        role="option"
        tabIndex={-1}
        type="button"
      />
      <ProfileAvatar
        avatarUrl={user.avatarUrl}
        className="pointer-events-none relative z-10 h-8 w-8 text-xs shadow-none"
        iconClassName="h-4 w-4"
        label={name}
      />
      <div className="pointer-events-none relative z-10 min-w-0 flex-1">
        {user.isAgent ? (
          <div className="min-w-0">
            <div className="flex min-w-0 items-center gap-2">
              <span className="truncate text-sm font-medium tracking-tight">
                {name}
              </span>
              <span className="inline-flex shrink-0 items-center gap-1 text-xs text-muted-foreground">
                <Bot
                  aria-hidden="true"
                  className="h-3 w-3"
                  data-testid="new-dm-agent-icon"
                />
                agent
              </span>
            </div>
            {ownerLabel ? (
              <span className="block truncate text-xs text-muted-foreground">
                owned by {ownerLabel}
              </span>
            ) : null}
            <span
              className="hidden min-w-0 break-all font-mono text-2xs leading-snug text-muted-foreground group-hover/dm-result:block group-focus-within/dm-result:block"
              data-testid={`new-dm-npub-${user.pubkey}`}
            >
              {safeNpub(user.pubkey) ?? truncatePubkey(user.pubkey)}
            </span>
          </div>
        ) : (
          <span className="block truncate text-sm font-medium tracking-tight">
            {name}
          </span>
        )}
      </div>
    </div>
  );
}
