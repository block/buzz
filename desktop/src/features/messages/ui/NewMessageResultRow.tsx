import { Bot } from "lucide-react";

import { formatOwnerLabel } from "@/features/profile/lib/identity";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { UserSearchResult } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { truncatePubkey } from "@/shared/lib/pubkey";

import { formatRecipientName } from "./useNewMessageRecipients";

const RESULT_ROW_INSET_DIVIDER_CLASS =
  "after:pointer-events-none after:absolute after:bottom-0 after:left-[3.75rem] after:right-0 after:h-px after:bg-border/60 after:content-[''] last:after:hidden";
const TEXT_SWAP_BASE_CLASS =
  "min-w-0 truncate transition-[opacity,filter] duration-[250ms] ease-in-out motion-reduce:transition-none";
const TEXT_SWAP_VISIBLE_CLASS = "opacity-100 blur-0";
const TEXT_SWAP_HIDDEN_CLASS = "opacity-0 blur-0";
const TEXT_SWAP_HOVER_VISIBLE_CLASS =
  "group-hover/name:opacity-100 group-hover/name:blur-0 group-focus-visible/dm-result:opacity-100 group-focus-visible/dm-result:blur-0";
const TEXT_SWAP_HOVER_HIDDEN_CLASS =
  "group-hover/name:opacity-0 group-hover/name:blur-[2px] group-focus-visible/dm-result:opacity-0 group-focus-visible/dm-result:blur-[2px]";

function HoverRecipientIdentity({
  displayName,
  pubkey,
}: {
  displayName: string;
  pubkey: string;
}) {
  const identityLabel = truncatePubkey(pubkey);

  return (
    <span
      className="group/name relative inline-flex h-5 min-w-0 max-w-full self-start leading-5"
      data-testid={`new-dm-name-${pubkey}`}
    >
      <span
        className={cn(
          TEXT_SWAP_BASE_CLASS,
          TEXT_SWAP_VISIBLE_CLASS,
          TEXT_SWAP_HOVER_HIDDEN_CLASS,
          "w-fit max-w-full text-sm font-medium tracking-tight",
        )}
      >
        {displayName}
      </span>
      <span
        className={cn(
          TEXT_SWAP_BASE_CLASS,
          TEXT_SWAP_HIDDEN_CLASS,
          TEXT_SWAP_HOVER_VISIBLE_CLASS,
          "absolute inset-y-0 left-0 font-mono text-2xs text-muted-foreground",
        )}
        data-testid={`new-dm-npub-${pubkey}`}
      >
        {identityLabel}
      </span>
    </span>
  );
}

/**
 * A single selectable person/agent row in the new-message directory. Extracted
 * from the former NewDirectMessageDialog so the compose page renders identical
 * rows (avatar, agent badge, owner label, and a name-to-pubkey hover swap).
 */
export function NewMessageResultRow({
  currentPubkey,
  disabled,
  isAlreadySelected = false,
  isKeyboardHighlighted = false,
  onSelect,
  ownerProfiles,
  user,
}: {
  currentPubkey?: string;
  disabled: boolean;
  isAlreadySelected?: boolean;
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
      className={cn("relative", RESULT_ROW_INSET_DIVIDER_CLASS)}
      data-keyboard-highlighted={isKeyboardHighlighted ? "true" : undefined}
    >
      <button
        aria-label={`${isAlreadySelected ? "Already added" : "Add"} ${name}`}
        aria-selected={isAlreadySelected || isKeyboardHighlighted}
        className={cn(
          "group/dm-result flex min-h-14 w-full cursor-pointer items-center gap-3 px-4 py-3.5 text-left transition-colors duration-150 ease-out hover:bg-muted/40 focus-visible:bg-muted/40 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60",
          isKeyboardHighlighted && "bg-muted/40",
        )}
        data-testid={`new-dm-result-${user.pubkey}`}
        disabled={disabled}
        id={`new-dm-option-${user.pubkey}`}
        onClick={() => onSelect(user)}
        role="option"
        tabIndex={-1}
        type="button"
      >
        <ProfileAvatar
          avatarUrl={user.avatarUrl}
          className="h-8 w-8 text-xs shadow-none"
          iconClassName="h-4 w-4"
          label={name}
        />
        <div className="min-w-0 flex-1">
          {user.isAgent ? (
            <div className="min-w-0">
              <div className="flex min-w-0 items-center gap-2">
                <div className="flex min-w-0 flex-1">
                  <HoverRecipientIdentity
                    displayName={name}
                    pubkey={user.pubkey}
                  />
                </div>
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
                  managed by {ownerLabel}
                </span>
              ) : null}
            </div>
          ) : (
            <HoverRecipientIdentity displayName={name} pubkey={user.pubkey} />
          )}
        </div>
      </button>
    </div>
  );
}
