import type { ReactNode } from "react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { cn } from "@/shared/lib/cn";
import { IdentityInitialsAvatar } from "./IdentityInitialsAvatar";

type AgentIdentityCardProps = {
  actions?: ReactNode;
  ariaLabel: string;
  avatar?: ReactNode;
  avatarUrl?: string | null;
  dataTestId: string;
  label: string;
  modelLabel?: string | null;
  onClick: () => void;
  /** Optional badge rendered below the label (e.g. "Restart required"). */
  statusBadge?: ReactNode;
};

export function AgentIdentityCard({
  actions,
  ariaLabel,
  avatar,
  avatarUrl,
  dataTestId,
  label,
  modelLabel,
  onClick,
  statusBadge,
}: AgentIdentityCardProps) {
  const trimmedAvatarUrl = avatarUrl?.trim() || null;

  return (
    <div
      className={cn(
        "group relative aspect-[4/5] w-full min-w-0 overflow-hidden rounded-2xl border border-border/70 bg-muted/50 text-left shadow-xs transition-colors hover:border-border hover:bg-muted/65",
      )}
      data-testid={dataTestId}
    >
      <button
        aria-label={ariaLabel}
        className="absolute inset-0 z-10 rounded-2xl focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
        onClick={onClick}
        type="button"
      />

      <div className="pointer-events-none relative z-20 flex h-full w-full min-w-0 flex-col items-center justify-center gap-5 px-4 pb-12 text-center">
        <div className="flex h-24 w-24 items-center justify-center">
          {avatar ??
            (trimmedAvatarUrl ? (
              <ProfileAvatar
                avatarUrl={trimmedAvatarUrl}
                className="h-full w-full border-[3px] border-background bg-muted shadow-none"
                iconClassName="h-8 w-8"
                label={label}
              />
            ) : (
              <IdentityInitialsAvatar
                className="shadow-none"
                label={label}
                size={96}
              />
            ))}
        </div>
      </div>

      {actions ? (
        <div className="absolute top-3 right-3 z-40">{actions}</div>
      ) : null}

      <div className="pointer-events-none absolute right-3 bottom-3 left-3 z-30 flex min-w-0 flex-col gap-0.5 text-left text-sm leading-5">
        <span className="min-w-0 truncate font-semibold text-foreground tracking-normal">
          {label}
        </span>
        {modelLabel ? (
          <span className="min-w-0 truncate text-xs font-normal text-secondary-foreground/75">
            {modelLabel}
          </span>
        ) : null}
        {statusBadge}
      </div>
    </div>
  );
}
