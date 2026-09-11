import * as React from "react";

import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";

/** Matches Tailwind `-space-x-1.5` so stacked rows stay zoom-safe. */
const STACK_OVERLAP_REM = 0.375;

/** How many overlapping avatars fit in a wrapping stack before the +N chip. */
export function visibleStackedAvatarCount({
  avatarSize,
  containerWidth,
  maxRows = 2,
  overlap,
  total,
}: {
  avatarSize: number;
  containerWidth: number;
  maxRows?: number;
  overlap: number;
  total: number;
}): { columns: number; remaining: number; visible: number } {
  if (total <= 0) return { columns: 1, remaining: 0, visible: 0 };
  if (containerWidth <= 0 || avatarSize <= 0) {
    return { columns: total, remaining: 0, visible: total };
  }
  const stride = Math.max(1, avatarSize - overlap);
  const columns = Math.max(
    1,
    1 + Math.floor(Math.max(0, containerWidth - avatarSize) / stride),
  );
  const slots = columns * maxRows;
  if (total <= slots) return { columns, remaining: 0, visible: total };
  const visible = Math.max(1, slots - 1);
  return { columns, remaining: total - visible, visible };
}

function OverviewPerson({
  index,
  profiles,
  pubkey,
  stackSize,
}: {
  index: number;
  profiles?: UserProfileLookup;
  pubkey: string;
  stackSize: number;
}) {
  const profile = profiles?.[normalizePubkey(pubkey)];
  const label = resolveUserLabel({ profiles, pubkey });
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          className={cn(
            "relative inline-flex ring-2 ring-background",
            profile?.isAgent ? "rounded-[30%]" : "rounded-full",
          )}
          data-overview-person=""
          style={{ zIndex: stackSize - index }}
        >
          <UserAvatar
            accent={profile?.isAgent === true}
            avatarUrl={profile?.avatarUrl ?? null}
            displayName={label}
            shape={profile?.isAgent ? "squircle" : "circle"}
            size="sm"
          />
        </span>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

export function ProjectsOverviewPeople({
  className,
  people,
  profiles,
}: {
  className?: string;
  people: string[];
  profiles?: UserProfileLookup;
}) {
  const containerRef = React.useRef<HTMLDivElement>(null);
  const [layout, setLayout] = React.useState({
    columns: Math.min(people.length, 48) || 1,
    visible: Math.min(people.length, 48),
  });

  React.useLayoutEffect(() => {
    const container = containerRef.current;
    if (!container || people.length === 0) return;

    function measure() {
      const node = containerRef.current;
      if (!node) return;
      const probe = node.querySelector(
        "[data-overview-person]",
      ) as HTMLElement | null;
      const avatarSize = probe?.offsetWidth ?? 24;
      const rootSize = Number.parseFloat(
        getComputedStyle(document.documentElement).fontSize,
      );
      const next = visibleStackedAvatarCount({
        avatarSize,
        containerWidth: node.clientWidth,
        overlap: STACK_OVERLAP_REM * rootSize,
        total: people.length,
      });
      setLayout((current) =>
        current.columns === next.columns && current.visible === next.visible
          ? current
          : { columns: next.columns, visible: next.visible },
      );
    }

    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(container);
    return () => observer.disconnect();
  }, [people.length]);

  if (people.length === 0) return null;

  const visible = people.slice(0, layout.visible);
  const remaining = people.length - visible.length;
  const remainingLabels = people
    .slice(visible.length)
    .map((pubkey) => resolveUserLabel({ profiles, pubkey }));
  const rows: string[][] = [];
  for (let index = 0; index < visible.length; index += layout.columns) {
    rows.push(visible.slice(index, index + layout.columns));
  }

  return (
    <div
      className={cn("flex flex-col gap-1", className)}
      data-testid="projects-overview-people"
      ref={containerRef}
    >
      {rows.map((row, rowIndex) => {
        const isLastRow = rowIndex === rows.length - 1;
        const showOverflow = isLastRow && remaining > 0;
        const stackSize = row.length + (showOverflow ? 1 : 0);
        return (
          <div className="flex -space-x-1.5" key={row[0] ?? rowIndex}>
            {row.map((pubkey, index) => (
              <OverviewPerson
                index={index}
                key={pubkey}
                profiles={profiles}
                pubkey={pubkey}
                stackSize={stackSize}
              />
            ))}
            {showOverflow ? (
              <Tooltip>
                <TooltipTrigger asChild>
                  <span
                    className="relative z-0 inline-flex h-6 min-w-6 items-center justify-center rounded-full bg-muted px-1 text-3xs font-semibold tabular-nums text-muted-foreground ring-2 ring-background"
                    data-testid="projects-overview-people-overflow"
                  >
                    +{remaining}
                  </span>
                </TooltipTrigger>
                <TooltipContent>
                  {remainingLabels.length === 1
                    ? remainingLabels[0]
                    : `${remaining} more`}
                </TooltipContent>
              </Tooltip>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
