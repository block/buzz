import { MessageSquare } from "lucide-react";
import type * as React from "react";

import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { relativeTime } from "@/features/projects/lib/projectsViewHelpers";
import { cn } from "@/shared/lib/cn";
import { UserAvatar } from "@/shared/ui/UserAvatar";

/** One-line list row: title, optional description column, then trailing metadata. */
export const PROJECT_ENTITY_LIST_ROW_CLASS =
  "flex min-h-9 w-full min-w-0 items-center gap-3 px-4 py-1.5 text-left transition-colors hover:bg-muted/30";

export function ProjectEntityFacepile({
  interactive = false,
  participants,
  profiles,
}: {
  /** Wrap each avatar in a profile popover. Leave off when the facepile is
   * nested inside another button (nested interactive elements are invalid). */
  interactive?: boolean;
  participants: string[];
  profiles: UserProfileLookup | undefined;
}) {
  const shown = participants.slice(0, 4);
  const overflow = participants.length - shown.length;
  if (shown.length === 0) return null;
  return (
    <span className="flex shrink-0 items-center">
      {shown.map((pubkey, index) => {
        const label = resolveUserLabel({ profiles, pubkey });
        if (!interactive) {
          return (
            <span
              className={cn(index > 0 && "-ml-1.5")}
              key={pubkey}
              title={label}
            >
              <UserAvatar
                avatarUrl={profiles?.[pubkey]?.avatarUrl ?? null}
                className="rounded-full ring-2 ring-background"
                displayName={label}
                size="xs"
              />
            </span>
          );
        }
        return (
          <UserProfilePopover
            key={pubkey}
            pubkey={pubkey}
            triggerElement="span"
          >
            <button
              className={cn("rounded-full", index > 0 && "-ml-1.5")}
              title={label}
              type="button"
            >
              <UserAvatar
                avatarUrl={profiles?.[pubkey]?.avatarUrl ?? null}
                className="rounded-full ring-2 ring-background"
                displayName={label}
                size="xs"
              />
            </button>
          </UserProfilePopover>
        );
      })}
      {overflow > 0 ? (
        <span className="-ml-1.5 flex h-5 w-5 items-center justify-center rounded-full bg-muted text-3xs text-muted-foreground ring-2 ring-background">
          +{overflow}
        </span>
      ) : null}
    </span>
  );
}

export function ProjectEntityListRow({
  affiliation,
  affiliationTestId,
  affiliationTitle,
  count,
  countSuffix,
  countTestId,
  countTitle,
  dateSeconds,
  dateTestId,
  description,
  descriptionTestId,
  icon,
  onClick,
  people,
  peopleSlot,
  peopleTestId,
  profiles,
  testId,
  title,
  titleAttr,
  trailing,
}: {
  affiliation?: React.ReactNode;
  affiliationTestId?: string;
  affiliationTitle?: string;
  count?: number | null;
  countSuffix?: string;
  countTestId?: string;
  countTitle?: string;
  dateSeconds?: number | null;
  dateTestId?: string;
  description?: string;
  descriptionTestId?: string;
  icon: React.ReactNode;
  onClick?: () => void;
  people?: string[];
  peopleSlot?: React.ReactNode;
  peopleTestId?: string;
  profiles?: UserProfileLookup;
  testId?: string;
  title: React.ReactNode;
  titleAttr?: string;
  trailing?: React.ReactNode;
}) {
  const peopleContent =
    peopleSlot ??
    (people && people.length > 0 ? (
      <ProjectEntityFacepile
        interactive={Boolean(trailing)}
        participants={people}
        profiles={profiles}
      />
    ) : null);

  const interactiveSlotClass = trailing ? "pointer-events-auto" : undefined;
  const body = (
    <>
      <span
        className={cn(
          "flex h-4 w-4 shrink-0 items-center justify-center",
          interactiveSlotClass,
        )}
      >
        {icon}
      </span>
      <span
        className={cn(
          "min-w-0 truncate text-sm font-medium text-foreground",
          description ? "flex-1 lg:w-44 lg:flex-none lg:shrink-0" : "flex-1",
        )}
      >
        {title}
      </span>
      {description ? (
        <span
          className="hidden min-w-0 flex-1 truncate text-xs text-muted-foreground lg:block"
          data-testid={descriptionTestId}
          title={description}
        >
          {description}
        </span>
      ) : null}
      {affiliation ? (
        <span
          className="hidden w-36 shrink-0 truncate text-right text-xs text-muted-foreground md:block"
          data-testid={affiliationTestId}
          title={
            affiliationTitle ??
            (typeof affiliation === "string" ? affiliation : undefined)
          }
        >
          {affiliation}
        </span>
      ) : null}
      <span
        className={cn("flex w-24 shrink-0 justify-end", interactiveSlotClass)}
        data-testid={peopleTestId}
      >
        {peopleContent}
      </span>
      {count != null ? (
        <span
          className="flex w-12 shrink-0 items-center justify-end gap-1 text-xs text-muted-foreground"
          data-testid={countTestId}
          title={countTitle}
        >
          <MessageSquare className="h-3.5 w-3.5" />
          {count}
          {countSuffix}
        </span>
      ) : null}
      {dateSeconds ? (
        <span
          className="hidden w-24 shrink-0 whitespace-nowrap text-right text-xs text-muted-foreground/70 sm:block"
          data-testid={dateTestId}
          title={new Date(dateSeconds * 1_000).toLocaleString()}
        >
          {relativeTime(dateSeconds)}
        </span>
      ) : (
        <span className="hidden w-24 shrink-0 sm:block" />
      )}
      {trailing ? (
        <span className="pointer-events-auto relative z-10 shrink-0">
          {trailing}
        </span>
      ) : null}
    </>
  );

  if (trailing) {
    return (
      <div className="group relative" data-testid={testId}>
        {onClick ? (
          <button
            className="absolute inset-0"
            onClick={onClick}
            title={titleAttr}
            type="button"
          >
            <span className="sr-only">{titleAttr}</span>
          </button>
        ) : null}
        <div
          className={cn(
            PROJECT_ENTITY_LIST_ROW_CLASS,
            "pointer-events-none relative z-10 group-hover:bg-muted/30",
          )}
        >
          {body}
        </div>
      </div>
    );
  }

  if (onClick) {
    return (
      <button
        className={PROJECT_ENTITY_LIST_ROW_CLASS}
        data-testid={testId}
        onClick={onClick}
        title={titleAttr}
        type="button"
      >
        {body}
      </button>
    );
  }

  return (
    <div className={PROJECT_ENTITY_LIST_ROW_CLASS} data-testid={testId}>
      {body}
    </div>
  );
}
