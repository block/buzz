import type { ReactNode } from "react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { cn } from "@/shared/lib/cn";
import { IdentityInitialsAvatar } from "./IdentityInitialsAvatar";

type AgentIdentityCardProps = {
  actions?: ReactNode;
  ariaLabel: string;
  avatar?: ReactNode;
  avatarUrl?: string | null;
  compact?: boolean;
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
  compact = false,
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
        compact
          ? "group relative flex min-h-16 w-full min-w-0 items-center overflow-hidden rounded-xl border border-border/70 bg-muted/50 text-left shadow-xs transition-colors hover:border-border hover:bg-muted/65"
          : "group relative aspect-[4/5] w-full min-w-0 overflow-hidden rounded-2xl border border-border/70 bg-muted/50 text-left shadow-xs transition-colors hover:border-border hover:bg-muted/65",
      )}
      data-testid={dataTestId}
    >
      <button
        aria-label={ariaLabel}
        className="absolute inset-0 z-10 rounded-xl focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
        onClick={onClick}
        type="button"
      />

      <div
        className={cn(
          "pointer-events-none relative z-20 flex min-w-0",
          compact
            ? "h-16 w-full items-center gap-3 px-3 text-left"
            : "h-full w-full flex-col items-center justify-center gap-5 px-4 pb-12 text-center",
        )}
      >
        <div
          className={cn(
            "flex shrink-0 items-center justify-center",
            compact ? "h-10 w-10" : "h-24 w-24",
          )}
        >
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

      <div
        className={cn(
          "pointer-events-none z-30 flex min-w-0 flex-col gap-0.5 text-left text-sm leading-5",
          compact
            ? "relative right-auto bottom-auto left-auto flex-1 pr-10"
            : "absolute right-3 bottom-3 left-3",
        )}
      >
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
