import { Check, MessageSquare, Minus } from "lucide-react";
import type * as React from "react";

import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import type { ProjectSelectionItem } from "@/features/projects/lib/projectSelection";
import { relativeTime } from "@/features/projects/lib/projectsViewHelpers";
import { useProjectSelection } from "@/features/projects/lib/useProjectSelection";
import { cn } from "@/shared/lib/cn";
import { UserAvatar } from "@/shared/ui/UserAvatar";

/** One-line list row: title, optional description column, then trailing metadata. */
export const PROJECT_ENTITY_LIST_ROW_CLASS =
  "mx-2 flex min-h-9 min-w-0 items-center gap-3 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-muted/30";

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
        <span className="-ml-1.5 flex h-5 w-5 items-center justify-center rounded-full bg-muted text-3xs text-muted-foreground/65 ring-2 ring-background">
          +{overflow}
        </span>
      ) : null}
    </span>
  );
}

export function ProjectEntitySelectControl({
  checked,
  indeterminate = false,
  label = "Select",
  onToggle,
  testId = "projects-row-select",
}: {
  checked: boolean;
  indeterminate?: boolean;
  label?: string;
  onToggle: (event: { shiftKey: boolean }) => void;
  testId?: string;
}) {
  return (
    <label
      className={cn(
        "pointer-events-auto relative flex h-3.5 w-3.5 shrink-0 cursor-pointer items-center justify-center rounded-xs border focus-within:ring-2 focus-within:ring-ring focus-within:ring-offset-1",
        checked || indeterminate
          ? "border-primary bg-primary text-primary-foreground"
          : "border-muted-foreground/55 bg-background",
      )}
    >
      <input
        aria-label={label}
        checked={checked}
        className="absolute inset-0 z-10 cursor-pointer opacity-0"
        data-testid={testId}
        onChange={(event) => {
          const mouse = event.nativeEvent;
          onToggle({
            shiftKey: "shiftKey" in mouse ? Boolean(mouse.shiftKey) : false,
          });
        }}
        onClick={(event) => event.stopPropagation()}
        onPointerDown={(event) => event.stopPropagation()}
        ref={(input) => {
          if (input) input.indeterminate = indeterminate;
        }}
        type="checkbox"
      />
      {checked ? (
        <Check className="h-2.5 w-2.5" strokeWidth={3} />
      ) : indeterminate ? (
        <Minus className="h-2.5 w-2.5" strokeWidth={3} />
      ) : null}
    </label>
  );
}

export function ProjectEntityListRow({
  affiliation,
  affiliationTestId,
  affiliationTitle,
  beforeDate,
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
  selection,
  testId,
  title,
  titleAttr,
  titleIcon,
  titleSecondary,
  titleSecondaryTestId,
  trailing,
}: {
  affiliation?: React.ReactNode;
  affiliationTestId?: string;
  affiliationTitle?: string;
  beforeDate?: React.ReactNode;
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
  selection?: {
    item: ProjectSelectionItem;
    rangeItems: ProjectSelectionItem[];
  };
  testId?: string;
  title: React.ReactNode;
  titleAttr?: string;
  titleIcon?: React.ReactNode;
  titleSecondary?: string;
  titleSecondaryTestId?: string;
  trailing?: React.ReactNode;
}) {
  const projectSelection = useProjectSelection();
  const selected = Boolean(
    selection && projectSelection?.isSelected(selection.item.id),
  );
  const showSelectControl = Boolean(selection && projectSelection && selected);
  const useOverlay = Boolean(trailing || selection);
  const peopleContent =
    peopleSlot ??
    (people && people.length > 0 ? (
      <ProjectEntityFacepile
        interactive={useOverlay}
        participants={people}
        profiles={profiles}
      />
    ) : null);

  const interactiveSlotClass = useOverlay ? "pointer-events-auto" : undefined;
  const body = (
    <>
      <span
        className={cn(
          "relative flex h-4 w-4 shrink-0 items-center justify-center",
          interactiveSlotClass,
        )}
        data-testid="project-entity-leading-icon"
      >
        <span
          className={cn(
            "flex items-center justify-center",
            selection && "group-hover:opacity-0 group-focus-within:opacity-0",
            showSelectControl && "opacity-0",
          )}
        >
          {icon}
        </span>
        {selection && projectSelection ? (
          <span
            className={cn(
              "absolute inset-0 flex items-center justify-center opacity-0 group-hover:opacity-100 group-focus-within:opacity-100",
              showSelectControl && "opacity-100",
            )}
          >
            <ProjectEntitySelectControl
              checked={selected}
              label={`Select ${selection.item.title}`}
              onToggle={({ shiftKey }) =>
                projectSelection.toggle(selection.item, {
                  rangeItems: selection.rangeItems,
                  shiftKey,
                })
              }
            />
          </span>
        ) : null}
      </span>
      <span
        className={cn(
          "min-w-0 text-sm font-medium text-foreground",
          description ? "flex-1 lg:w-44 lg:flex-none lg:shrink-0" : "flex-1",
        )}
        data-projects-text-priority="primary"
      >
        {titleSecondary ? (
          <span className="flex min-w-0 items-baseline gap-2 overflow-hidden">
            <span
              className="max-w-full shrink-0 truncate"
              data-testid="project-entity-title"
            >
              {title}
            </span>
            <span
              className="min-w-0 flex-1 truncate text-left font-normal text-muted-foreground/65"
              data-projects-text-priority="secondary"
              data-testid={titleSecondaryTestId}
              title={titleSecondary}
            >
              {titleSecondary}
            </span>
          </span>
        ) : (
          <span className="block truncate" data-testid="project-entity-title">
            {title}
          </span>
        )}
      </span>
      {titleIcon ? (
        <span
          className="flex h-4 w-4 shrink-0 items-center justify-center"
          data-testid="project-entity-title-icon"
        >
          {titleIcon}
        </span>
      ) : null}
      {description ? (
        <span
          className="hidden min-w-0 flex-1 truncate text-xs text-muted-foreground/65 lg:block"
          data-projects-text-priority="secondary"
          data-testid={descriptionTestId ?? "project-entity-description"}
          title={description}
        >
          {description}
        </span>
      ) : null}
      {affiliation ? (
        <span
          className="hidden w-36 shrink-0 truncate text-right text-xs text-muted-foreground/65 md:block"
          data-projects-text-priority="secondary"
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
          className="grid w-12 shrink-0 grid-cols-[0.875rem_1fr] items-center gap-1 text-xs text-muted-foreground/65"
          data-projects-text-priority="secondary"
          data-testid={countTestId}
          title={countTitle}
        >
          <MessageSquare className="h-3.5 w-3.5" />
          <span className="text-right tabular-nums">
            {count}
            {countSuffix}
          </span>
        </span>
      ) : null}
      {beforeDate ? (
        <span
          className="pointer-events-auto relative z-10 shrink-0"
          data-testid="project-entity-before-date"
        >
          {beforeDate}
        </span>
      ) : null}
      {dateSeconds ? (
        <span
          className="hidden w-24 shrink-0 whitespace-nowrap text-right text-xs text-muted-foreground/55 sm:block"
          data-projects-text-priority="secondary"
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

  if (useOverlay) {
    return (
      <div
        className="group relative"
        data-selected={selected ? "true" : undefined}
        data-testid={testId}
      >
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
            selected && "bg-muted/40",
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
