export function DayDivider({ label }: { label: string }) {
  return (
    <section
      aria-label={label}
      className="pointer-events-none sticky top-(--buzz-channel-content-top-padding,5.75rem) z-20 flex justify-center"
      data-testid="message-timeline-day-divider"
      data-day-label={label}
    >
      <p className="relative z-10 shrink-0 rounded-full border border-border/70 bg-background px-2.5 py-1 text-2xs font-medium tracking-[0.02em] text-muted-foreground/70">
        {label}
      </p>
    </section>
  );
}
