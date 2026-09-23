import { cn } from "@/shared/lib/cn";

/** Public path for the OpenClaw crab mark (also used by RuntimeIcon). */
export const OPENCLAW_WORKSPACE_BADGE_SRC = "/harness-logos/openclaw.svg";

type OpenClawWorkspaceBadgeProps = {
  className?: string;
  /** Pixel size of the badge shell (~2× presence-dot; callers pass 36 / 20). */
  size?: number;
};

/**
 * Corner / inline mark for agents with `useOpenClawWorkspace` opted in.
 * Does not require a live Settings grant — the flag alone means show.
 */
export function OpenClawWorkspaceBadge({
  className,
  size = 20,
}: OpenClawWorkspaceBadgeProps) {
  return (
    <span
      aria-label="OpenClaw workspace"
      className={cn(
        "inline-flex shrink-0 items-center justify-center rounded-full bg-background/90 shadow-sm ring-1 ring-border/70",
        className,
      )}
      data-testid="openclaw-workspace-badge"
      role="img"
      style={{ width: size, height: size }}
      title="OpenClaw workspace"
    >
      <img
        alt=""
        aria-hidden="true"
        className="h-[72%] w-[72%] object-contain"
        draggable={false}
        src={OPENCLAW_WORKSPACE_BADGE_SRC}
      />
    </span>
  );
}
