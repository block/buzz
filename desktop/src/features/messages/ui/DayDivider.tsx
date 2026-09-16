import { cn } from "@/shared/lib/cn";
import { channelChrome } from "@/shared/layout/chromeLayout";

export function DayDivider({
  label,
  sticky = true,
  testId = "message-timeline-day-divider",
}: {
  label: string;
  sticky?: boolean;
  testId?: string;
}) {
  return (
    <section
      aria-label={label}
      className={cn(
        sticky
          ? cn(
              "pointer-events-none sticky z-20 flex justify-center",
              channelChrome.stickyTimelineTop,
            )
          : "pointer-events-none flex justify-center",
      )}
      data-testid={testId}
      data-day-label={label}
    >
      <p className="relative z-10 shrink-0 rounded-full border border-border/70 bg-background px-2.5 py-1 text-2xs font-medium tracking-[0.02em] text-muted-foreground/70">
        {label}
      </p>
    </section>
  );
}
