import type { ReactNode } from "react";
import { Info, Link, Users } from "lucide-react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { AgentPersona } from "@/shared/api/types";
import { Card } from "@/shared/ui/card";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { formatAgentModelLabel } from "@/features/agents/lib/formatAgentModelLabel";
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
  teamName: string;
  version?: string | null;
};

const MAX_VISIBLE_MEMBER_AVATARS = 4;

export function TeamIdentityCard({
  actions,
  children,
  dataTestId,
  description,
  isSymlink = false,
  memberCount,
  personas,
  sourceDir,
  symlinkTarget,
  teamName,
  version,
}: TeamIdentityCardProps) {
  const footerModelLabel = getTeamFooterModelLabel(personas);
  const trimmedDescription = description?.trim();

  return (
    <Card
      className="min-w-0 overflow-hidden rounded-2xl p-0 transition-colors hover:border-border hover:bg-muted/65"
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
          {trimmedDescription ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  aria-label={`${teamName} description`}
                  className="flex h-6 w-6 items-center justify-center rounded-full border border-border/65 bg-background/90 text-muted-foreground shadow-xs"
                  type="button"
                >
                  <Info className="h-3.5 w-3.5" />
                </button>
              </TooltipTrigger>
              <TooltipContent side="bottom" className="max-w-xs">
                <p>{trimmedDescription}</p>
              </TooltipContent>
            </Tooltip>
          ) : null}
        </div>

        <div className="absolute top-3 right-3 z-40">{actions}</div>

        <TeamAvatarRow
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

function TeamAvatarRow({
  memberCount,
  personas,
  teamName,
}: {
  memberCount: number;
  personas: AgentPersona[];
  teamName: string;
}) {
  const visiblePersonas = personas.slice(0, MAX_VISIBLE_MEMBER_AVATARS);
  const overflowCount = Math.max(0, memberCount - visiblePersonas.length);
  const stackItemCount = visiblePersonas.length + (overflowCount > 0 ? 1 : 0);

  if (visiblePersonas.length === 0 && overflowCount === 0) {
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
        className="flex max-w-full items-center justify-center px-4"
        role="img"
      >
        {visiblePersonas.map((persona, index) => (
          <TeamAvatarItem
            index={index}
            isFollowedByAnother={index < stackItemCount - 1}
            key={persona.id}
            persona={persona}
          />
        ))}
        {overflowCount > 0 ? (
          <div
            className={visiblePersonas.length > 0 ? "-ml-5" : ""}
            style={{ zIndex: stackItemCount }}
          >
            <span className="flex h-14 w-14 items-center justify-center rounded-full bg-card text-sm font-semibold text-muted-foreground">
              +{overflowCount}
            </span>
          </div>
        ) : null}
      </div>
    </div>
  );
}

function TeamAvatarItem({
  index,
  isFollowedByAnother,
  persona,
}: {
  index: number;
  isFollowedByAnother: boolean;
  persona: AgentPersona;
}) {
  const avatarUrl = persona.avatarUrl?.trim() ?? null;

  return (
    <div
      className={`h-14 w-14 ${index > 0 ? "-ml-5" : ""}`}
      data-team-member-avatar="avatar"
      style={{
        zIndex: index + 1,
        ...(isFollowedByAnother && {
          mask: "radial-gradient(circle 32px at calc(100% + 8px) 50%, transparent 99%, #fff 100%)",
          WebkitMask:
            "radial-gradient(circle 32px at calc(100% + 8px) 50%, transparent 99%, #fff 100%)",
        }),
      }}
    >
      {avatarUrl ? (
        <ProfileAvatar
          avatarUrl={avatarUrl}
          className="h-full w-full bg-muted shadow-none"
          iconClassName="h-6 w-6"
          label={persona.displayName}
          testId={`team-member-avatar-${persona.id}`}
        />
      ) : (
        <IdentityInitialsAvatar
          className="border-0 shadow-none"
          colorIndex={index}
          label={persona.displayName}
          size={56}
        />
      )}
    </div>
  );
}

function getTeamFooterModelLabel(personas: AgentPersona[]) {
  const modelLabels = personas
    .map((persona) => formatAgentModelLabel(persona.model))
    .filter((model): model is string => Boolean(model));

  if (modelLabels.length === 0) return "Auto";

  const uniqueModels = new Map(
    modelLabels.map((model) => [model.toLowerCase(), model]),
  );

  return uniqueModels.size === 1
    ? (uniqueModels.values().next().value ?? "Auto")
    : "Mixed models";
}
