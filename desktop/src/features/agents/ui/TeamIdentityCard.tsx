import type { ReactNode } from "react";
import { Link, Users } from "lucide-react";

import type { AgentPersona } from "@/shared/api/types";
import { Card } from "@/shared/ui/card";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { IdentityInitialsAvatar } from "./IdentityInitialsAvatar";

type TeamIdentityCardProps = {
  actions: ReactNode;
  children?: ReactNode;
  dataTestId: string;
  description?: string | null;
  isSymlink?: boolean;
  memberCount: number;
  personas: AgentPersona[];
  sourceDir?: string | null;
  symlinkTarget?: string | null;
  teamId: string;
  teamName: string;
  version?: string | null;
};

type ClusterItem =
  | {
      kind: "persona";
      persona: AgentPersona;
    }
  | {
      count: number;
      kind: "overflow";
    };

const MAX_STACK_ITEMS = 6;

export function TeamIdentityCard({
  actions,
  children,
  dataTestId,
  isSymlink = false,
  memberCount,
  personas,
  sourceDir,
  symlinkTarget,
  teamName,
  version,
}: TeamIdentityCardProps) {
  const footerModelLabel = getTeamFooterModelLabel(personas);

  return (
    <Card
      className="min-w-0 overflow-hidden p-0 transition-colors hover:border-border hover:bg-muted/65"
      data-testid={dataTestId}
    >
      <div className="relative aspect-[4/5] min-w-0 overflow-hidden bg-muted/50">
        <div className="absolute top-3 left-3 z-30 flex max-w-[calc(100%-4rem)] flex-wrap items-center gap-1.5">
          {isSymlink ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="flex h-6 w-6 items-center justify-center rounded-full border border-border/65 bg-background/90 text-muted-foreground shadow-xs">
                  <Link className="h-3.5 w-3.5" />
                </span>
              </TooltipTrigger>
              <TooltipContent side="bottom" className="max-w-xs">
                <p>Linked from {symlinkTarget ?? sourceDir}</p>
              </TooltipContent>
            </Tooltip>
          ) : null}
          {version ? (
            <span className="rounded-full border border-border/65 bg-background/90 px-2 py-1 text-2xs font-medium leading-none text-muted-foreground shadow-xs">
              v{version}
            </span>
          ) : null}
        </div>

        <div className="absolute top-3 right-3 z-40">{actions}</div>

        <TeamAvatarCluster
          memberCount={memberCount}
          personas={personas}
          teamName={teamName}
        />

        <div className="absolute right-3 bottom-3 left-3 z-30 flex min-w-0 flex-col gap-0.5 text-left text-sm leading-5">
          <span className="min-w-0 truncate font-semibold tracking-normal text-foreground">
            {teamName}
          </span>
          <span className="min-w-0 truncate font-normal text-secondary-foreground/75">
            {footerModelLabel}
          </span>
        </div>
      </div>
      {children}
    </Card>
  );
}

function TeamAvatarCluster({
  memberCount,
  personas,
  teamName,
}: {
  memberCount: number;
  personas: AgentPersona[];
  teamName: string;
}) {
  const items = buildClusterItems(personas, memberCount);
  const { maskOffset, maskRadius, overlap, size } = getStackMetrics(
    items.length,
  );

  if (items.length === 0) {
    return (
      <div className="absolute inset-x-4 top-0 bottom-12 flex items-center justify-center">
        <div className="flex h-24 w-24 items-center justify-center rounded-full border border-border/65 bg-background/80 text-muted-foreground shadow-xs">
          <Users className="h-9 w-9" />
        </div>
      </div>
    );
  }

  return (
    <div className="absolute inset-x-0 top-0 bottom-12 flex items-center justify-center">
      <div
        aria-label={`${teamName} member avatars`}
        className="flex max-w-full items-center justify-center px-2"
        role="img"
      >
        {items.map((item, index) => (
          <div
            data-team-cluster-item="avatar"
            key={item.kind === "persona" ? item.persona.id : "overflow"}
            style={{
              height: size,
              marginLeft: index > 0 ? -overlap : 0,
              width: size,
              zIndex: index + 1,
              ...(index < items.length - 1
                ? {
                    WebkitMask: `radial-gradient(circle ${maskRadius}px at calc(100% + ${maskOffset}px) 50%, transparent 99%, #fff 100%)`,
                    mask: `radial-gradient(circle ${maskRadius}px at calc(100% + ${maskOffset}px) 50%, transparent 99%, #fff 100%)`,
                  }
                : null),
            }}
          >
            {item.kind === "persona" ? (
              <IdentityInitialsAvatar
                colorIndex={index}
                label={item.persona.displayName}
                size={size}
              />
            ) : (
              <span className="flex h-full w-full items-center justify-center rounded-full border-[3px] border-background bg-card text-base font-semibold text-muted-foreground shadow-sm">
                +{item.count}
              </span>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}

function buildClusterItems(
  personas: AgentPersona[],
  memberCount: number,
): ClusterItem[] {
  const hasOverflow = memberCount > MAX_STACK_ITEMS;
  const visibleLimit = hasOverflow ? MAX_STACK_ITEMS - 1 : MAX_STACK_ITEMS;
  const visiblePersonas = personas.slice(0, visibleLimit);
  const overflowCount = memberCount - visiblePersonas.length;

  return [
    ...visiblePersonas.map((persona) => ({
      kind: "persona" as const,
      persona,
    })),
    ...(overflowCount > 0
      ? [{ count: overflowCount, kind: "overflow" as const }]
      : []),
  ].slice(0, MAX_STACK_ITEMS);
}

function getStackMetrics(count: number) {
  switch (Math.max(1, Math.min(count, MAX_STACK_ITEMS))) {
    case 1:
      return { maskOffset: 18, maskRadius: 84, overlap: 0, size: 152 };
    case 2:
      return { maskOffset: 18, maskRadius: 66, overlap: 44, size: 124 };
    case 3:
      return { maskOffset: 13, maskRadius: 62, overlap: 46, size: 108 };
    case 4:
      return { maskOffset: 12, maskRadius: 56, overlap: 50, size: 96 };
    case 5:
      return { maskOffset: 11, maskRadius: 51, overlap: 48, size: 86 };
    default:
      return { maskOffset: 10, maskRadius: 46, overlap: 46, size: 78 };
  }
}

function getTeamFooterModelLabel(personas: AgentPersona[]) {
  const modelLabels = personas
    .map((persona) => formatFooterModelLabel(persona.model))
    .filter((model): model is string => Boolean(model));

  if (modelLabels.length === 0) return "Auto";

  const uniqueModels = new Map(
    modelLabels.map((model) => [model.toLowerCase(), model]),
  );

  return uniqueModels.size === 1
    ? (uniqueModels.values().next().value ?? "Auto")
    : "Mixed models";
}

function formatFooterModelLabel(model: string | null | undefined) {
  const trimmed = model?.trim();
  return trimmed && trimmed.length > 0 ? trimmed : "Auto";
}
