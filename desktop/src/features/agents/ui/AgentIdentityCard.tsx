import { cn } from "@/shared/lib/cn";
import { AgentProviderCollage } from "./AgentProviderCollage";

type AgentIdentityCardProps = {
  ariaLabel: string;
  dataTestId: string;
  label: string;
  modelLabel: string;
  onClick: () => void;
};

export function AgentIdentityCard({
  ariaLabel,
  dataTestId,
  label,
  modelLabel,
  onClick,
}: AgentIdentityCardProps) {
  return (
    <button
      aria-label={ariaLabel}
      className={cn(
        "group relative flex aspect-[4/5] w-full min-w-0 overflow-hidden rounded-xl border border-border/70 bg-muted/50 text-left shadow-xs transition-colors hover:border-border hover:bg-muted/65 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
      )}
      data-testid={dataTestId}
      onClick={onClick}
      type="button"
    >
      <AgentProviderCollage label={label} />

      <div className="absolute right-3 bottom-3 left-3 z-30 flex min-w-0 flex-col gap-0.5 text-left text-sm leading-5">
        <span className="min-w-0 truncate font-semibold text-foreground tracking-normal">
          {label}
        </span>
        <span className="min-w-0 truncate font-normal text-secondary-foreground/75">
          {modelLabel}
        </span>
      </div>
    </button>
  );
}
