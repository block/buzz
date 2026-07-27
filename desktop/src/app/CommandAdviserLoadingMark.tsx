import appIcon from "@/assets/command-adviser/command-adviser-app-icon.png";
import { cn } from "@/shared/lib/cn";

export function CommandAdviserLoadingMark({
  className,
  compact = false,
}: {
  className?: string;
  compact?: boolean;
}) {
  return (
    <div
      aria-label="Command Adviser"
      className={cn(
        "relative z-10 flex flex-col items-center text-center",
        compact ? "gap-2" : "gap-3",
        className,
      )}
      role="img"
    >
      <img
        alt=""
        className={cn(
          "rounded-[22%] shadow-xl",
          compact ? "h-16 w-16" : "h-24 w-24",
        )}
        src={appIcon}
      />
      <p
        className={cn(
          "font-semibold tracking-wide",
          compact ? "text-sm" : "text-base",
        )}
      >
        Command Adviser
      </p>
      {compact ? null : (
        <p className="text-xs uppercase tracking-widest text-muted-foreground">
          Strengthen the Shield
        </p>
      )}
    </div>
  );
}
