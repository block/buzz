export function DayDivider({ label }: { label: string }) {
  return (
    <section
      aria-label={label}
      className="sticky top-[92px] z-[5] flex justify-center py-1 before:absolute before:inset-x-0 before:top-1/2 before:h-px before:-translate-y-1/2 before:bg-border/35 before:content-['']"
      data-testid="message-timeline-day-divider"
      data-day-label={label}
    >
      <p className="relative z-10 shrink-0 rounded-lg border border-border/70 bg-background/95 px-2 py-1 text-2xs font-medium tracking-[0.02em] text-muted-foreground/70 shadow-xs backdrop-blur-sm">
        {label}
      </p>
    </section>
  );
}
