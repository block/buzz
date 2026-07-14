import { Bot, X } from "lucide-react";

import type { UserSearchResult } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import {
  POOF_ORIGIN_CLASS,
  POOF_POINTER_ORIGIN_CLASS,
  POOF_TRIGGER_CLASS,
} from "@/shared/ui/PoofBurstProvider";
import { PubKey } from "@/shared/ui/PubKey";
import { Popover, PopoverAnchor, PopoverContent } from "@/shared/ui/popover";

import { ProfileAvatar } from "./ProfileAvatar";

type SelectedRecipientChipTestIds = {
  chip?: string;
  keyPopover?: string;
  name?: string;
  pubkey?: string;
};

/**
 * The selected-recipient pill shared by recipient pickers. Its avatar becomes
 * the remove affordance on hover/focus. Surfaces that need identity
 * verification can also make the name open the recipient's full key.
 */
export function SelectedRecipientChip({
  disabled,
  inspectable = true,
  inspectionOpen = false,
  label,
  onInspectionOpenChange,
  onRemove,
  testIds,
  user,
}: {
  disabled: boolean;
  inspectable?: boolean;
  inspectionOpen?: boolean;
  label: string;
  onInspectionOpenChange?: (open: boolean) => void;
  onRemove: () => void;
  testIds?: SelectedRecipientChipTestIds;
  user: UserSearchResult;
}) {
  return (
    <div className="inline-flex h-7 max-w-56 items-center gap-1.5 rounded-full bg-muted px-1.5 pr-2.5 text-sm transition-colors hover:bg-muted/80">
      <button
        aria-label={`Remove ${label}`}
        className={cn(
          "group/remove-recipient relative h-5 w-5 shrink-0 rounded-full focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60",
          POOF_TRIGGER_CLASS,
          POOF_ORIGIN_CLASS,
          POOF_POINTER_ORIGIN_CLASS,
        )}
        data-testid={testIds?.chip}
        disabled={disabled}
        onClick={(event) => {
          event.stopPropagation();
          onRemove();
        }}
        onPointerDown={(event) => {
          if (event.button !== 0) {
            return;
          }
          event.stopPropagation();
          onRemove();
        }}
        type="button"
      >
        <ProfileAvatar
          avatarUrl={user.avatarUrl}
          className="h-5 w-5 text-3xs shadow-none transition-opacity group-hover/remove-recipient:opacity-0 group-focus-visible/remove-recipient:opacity-0"
          iconClassName="h-2.5 w-2.5"
          label={label}
        />
        <span className="absolute inset-0 flex items-center justify-center rounded-full bg-foreground text-background opacity-0 transition-opacity group-hover/remove-recipient:opacity-100 group-focus-visible/remove-recipient:opacity-100">
          <X aria-hidden="true" className="h-3 w-3" />
        </span>
      </button>
      {inspectable ? (
        <Popover onOpenChange={onInspectionOpenChange} open={inspectionOpen}>
          <PopoverAnchor asChild>
            <button
              aria-expanded={inspectionOpen}
              aria-haspopup="dialog"
              aria-label={`Verify ${label} public key`}
              className="min-w-0 cursor-pointer truncate rounded font-medium hover:underline focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
              data-testid={testIds?.name}
              onClick={(event) => {
                event.stopPropagation();
                onInspectionOpenChange?.(!inspectionOpen);
              }}
              type="button"
            >
              {label}
            </button>
          </PopoverAnchor>
          <PopoverContent
            align="start"
            className="w-96 max-w-[90vw] space-y-2"
            data-recipient-key-popover=""
            data-testid={testIds?.keyPopover}
            onOpenAutoFocus={(event) => event.preventDefault()}
          >
            <p className="text-sm font-medium">Verify {label}</p>
            <PubKey
              pubkey={user.pubkey}
              testId={testIds?.pubkey}
              variant="full"
            />
            <p className="break-all font-mono text-xs text-muted-foreground">
              {user.pubkey}
            </p>
          </PopoverContent>
        </Popover>
      ) : (
        <span className="min-w-0 truncate font-medium">{label}</span>
      )}
      {user.isAgent ? (
        <Bot
          aria-label="agent"
          className="h-3.5 w-3.5 shrink-0 text-muted-foreground"
        />
      ) : null}
    </div>
  );
}
