import type { ReactNode } from "react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { cn } from "@/shared/lib/cn";
import { IdentityInitialsAvatar } from "./IdentityInitialsAvatar";

type AgentIdentityCardProps = {
  actions?: ReactNode;
  ariaLabel: string;
  avatarUrl?: string | null;
  dataTestId: string;
  label: string;
  errorLabel?: string | null;
  modelControl?: ReactNode;
  modelLabel: string;
  onClick: () => void;
  status?: ReactNode;
};

export function AgentIdentityCard({
  actions,
  ariaLabel,
  avatarUrl,
  dataTestId,
  errorLabel,
  label,
  modelControl,
  modelLabel,
  onClick,
  status,
}: AgentIdentityCardProps) {
  const trimmedAvatarUrl = avatarUrl?.trim() || null;

  return (
    <div
      className={cn(
        "group relative aspect-[4/5] w-full min-w-0 overflow-hidden rounded-xl border border-border/70 bg-muted/50 text-left shadow-xs transition-colors hover:border-border hover:bg-muted/65",
      )}
      data-testid={dataTestId}
    >
      <button
        aria-label={ariaLabel}
        className="flex h-full w-full min-w-0 flex-col items-center justify-center gap-5 px-4 pb-12 text-center focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
        onClick={onClick}
        type="button"
      >
        <div className="flex h-24 w-24 items-center justify-center">
          {trimmedAvatarUrl ? (
            <ProfileAvatar
              avatarUrl={trimmedAvatarUrl}
              className="h-full w-full border-[3px] border-background bg-muted shadow-sm"
              iconClassName="h-8 w-8"
              label={label}
            />
          ) : (
            <IdentityInitialsAvatar label={label} size={96} />
          )}
        </div>
      </button>

      {actions ? (
        <div className="absolute top-3 right-3 z-40">{actions}</div>
      ) : null}

      {status ? (
        <div className="absolute top-3 left-3 z-30 flex max-w-[calc(100%-4rem)] flex-wrap items-center gap-1.5">
          {status}
        </div>
      ) : null}

      <div className="absolute right-3 bottom-3 left-3 z-30 flex min-w-0 flex-col gap-0.5 text-left text-sm leading-5">
        <span className="min-w-0 truncate font-semibold text-foreground tracking-normal">
          {label}
        </span>
        {modelControl ?? (
          <span className="min-w-0 truncate font-normal text-secondary-foreground/75">
            {modelLabel}
          </span>
        )}
        {errorLabel ? (
          <span
            className="min-w-0 truncate text-2xs font-medium text-destructive"
            title={errorLabel}
          >
            {errorLabel}
          </span>
        ) : null}
      </div>
    </div>
  );
}
