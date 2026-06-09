export function DayDivider({ label }: { label: string }) {
  return (
    <section
      aria-label={label}
      className="sticky top-[92px] z-[5] flex justify-center py-1"
      data-testid="message-timeline-day-divider"
      data-day-label={label}
    >
      <p className="relative z-10 shrink-0 rounded-lg border border-border/70 bg-background/95 px-2 py-1 text-[10px] font-medium tracking-[0.02em] text-muted-foreground/70 shadow-xs backdrop-blur-sm">
        {label}
      </p>
    </section>
  );
}
