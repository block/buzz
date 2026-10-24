import type { ReactNode } from "react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { cn } from "@/shared/lib/cn";
import { IdentityInitialsAvatar } from "./IdentityInitialsAvatar";

type AgentIdentityCardProps = {
  actions?: ReactNode;
  ariaLabel: string;
  avatar?: ReactNode;
  avatarUrl?: string | null;
  footerAccessory?: ReactNode;
  dataTestId: string;
  label: string;
  /**
   * Second line under the agent name: the effective description when one
   * resolves (owner-authored — see `lib/agentDescription.ts`),
   * otherwise the model label. Callers compose the fallback.
   */
  subtitle?: string | null;
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
  footerAccessory,
  label,
  subtitle,
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
                shape="squircle"
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

      <div className="pointer-events-none absolute right-3 bottom-3 left-3 z-30 flex min-w-0 items-end gap-2 text-left text-sm leading-5">
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="min-w-0 truncate font-semibold text-foreground tracking-normal">
            {label}
          </span>
          {subtitle ? (
            <span className="line-clamp-2 min-w-0 text-xs font-normal text-muted-foreground">
              {subtitle}
            </span>
          ) : null}
          {/* pointer-events-auto: the overlay button above has pointer-events-none
              on this container, but the status badge itself (a sibling of the button
              in z-order) needs hover so the restart diff tooltip can fire. */}
          {statusBadge ? (
            <div className="pointer-events-auto">{statusBadge}</div>
          ) : null}
        </div>
        {footerAccessory ? (
          <div className="shrink-0">{footerAccessory}</div>
        ) : null}
      </div>
    </div>
  );
}
