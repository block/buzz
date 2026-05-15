export function DayDivider({ label }: { label: string }) {
  return (
    <section
      aria-label={label}
      className="sticky top-11 z-20 flex justify-center py-1"
      data-testid="message-timeline-day-divider"
      data-day-label={label}
    >
      <p className="shrink-0 rounded-full bg-background/90 px-2 py-0.5 text-[10px] font-medium tracking-[0.02em] text-muted-foreground/65 backdrop-blur-sm">
        {label}
      </p>
    </section>
  );
}
