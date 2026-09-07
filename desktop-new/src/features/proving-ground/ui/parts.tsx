import type { ReactNode } from "react";

import { type Health, HEALTH_LABEL, HEALTH_TONE } from "../data";

/**
 * Pieces the proving-ground dashboard is assembled from.
 *
 * These are page-local on purpose. The shared layer earns a component when the
 * product repeats one; this page exists to find out *which* things repeat, so
 * putting its parts in `shared/ui` first would be guessing. Anything that turns
 * out to be load-bearing graduates afterwards.
 */

type Tone = "success" | "warning" | "danger" | "neutral" | "accent" | "info";

/**
 * A status pill. Tint fill, coloured text — the tint arrangement from
 * DESIGN.md, where a colour means something rather than carrying an action.
 */
export function StatusPill({
  tone,
  children,
}: {
  tone: Tone;
  children: ReactNode;
}) {
  return (
    <span className="status-pill" data-tone={tone}>
      {children}
    </span>
  );
}

export function HealthPill({ health }: { health: Health }) {
  return (
    <StatusPill tone={HEALTH_TONE[health]}>{HEALTH_LABEL[health]}</StatusPill>
  );
}

/** A page region. Panel surface, generous inset, no card-per-row inside. */
export function Panel({
  title,
  description,
  action,
  children,
  padded = true,
}: {
  title: string;
  description?: string;
  action?: ReactNode;
  children: ReactNode;
  /** False when the content is edge-to-edge rows that supply their own inset. */
  padded?: boolean;
}) {
  return (
    <section className="dash-panel">
      <header className="dash-panel-header">
        <div className="flex min-w-0 flex-col gap-1">
          <h2 className="text-heading text-primary">{title}</h2>
          {description ? (
            <p className="text-body-sm text-secondary">{description}</p>
          ) : null}
        </div>
        {action}
      </header>
      <div data-padded={padded || undefined} className="dash-panel-body">
        {children}
      </div>
    </section>
  );
}

/**
 * A headline figure with its label and change.
 *
 * Hierarchy comes from size, not colour: the number is `text-display`, the label
 * is small and secondary. Only the change direction earns a colour, because a
 * direction is a signal.
 */
export function Metric({
  label,
  value,
  unit,
  change,
  direction,
}: {
  label: string;
  value: string;
  unit?: string;
  change?: string;
  direction?: "up" | "down" | "flat";
}) {
  return (
    <div className="dash-metric">
      <span className="text-body-sm text-secondary">{label}</span>
      <div className="flex items-baseline gap-1.5">
        <span className="text-display text-primary tabular-nums">{value}</span>
        {unit ? (
          <span className="text-body-sm text-tertiary">{unit}</span>
        ) : null}
      </div>
      {change ? (
        <span
          className="text-body-sm tabular-nums"
          data-direction={direction}
          // A rise is not automatically good, so direction is coloured by what
          // it means on this metric rather than by which way it points.
          style={{
            color:
              direction === "up"
                ? "var(--text-success)"
                : direction === "down"
                  ? "var(--text-danger)"
                  : "var(--text-tertiary)",
          }}
        >
          {change}
        </span>
      ) : null}
    </div>
  );
}

/**
 * A bar sparkline, drawn with divs rather than SVG.
 *
 * Deliberate: it inherits `currentColor` and the rem-based type scale, so it
 * tracks the theme and the font-size preference without a redraw. Decorative —
 * the figure beside it carries the information, so this is hidden from
 * assistive technology rather than described.
 */
export function Sparkline({
  values,
  tone = "accent",
}: {
  values: number[];
  tone?: Tone;
}) {
  const peak = Math.max(...values, 1);
  // A reading's position in the window is part of its identity — the oldest
  // sample is a different thing from the newest — so the key carries both.
  const bars = values.map((value, index) => ({
    id: `${index}:${value}`,
    height: `${Math.max((value / peak) * 100, 4)}%`,
  }));
  return (
    <div className="dash-sparkline" data-tone={tone} aria-hidden="true">
      {bars.map((bar) => (
        <span
          key={bar.id}
          className="dash-sparkline-bar"
          style={{ height: bar.height }}
        />
      ))}
    </div>
  );
}

/** A dense table row. Dividers, edge to edge — never a card per row. */
export function Row({
  children,
  interactive = false,
}: {
  children: ReactNode;
  interactive?: boolean;
}) {
  return (
    <div className="dash-row" data-interactive={interactive || undefined}>
      {children}
    </div>
  );
}

export function KeyValue({
  label,
  value,
}: {
  label: string;
  value: ReactNode;
}) {
  return (
    <div className="flex items-baseline justify-between gap-4">
      <span className="text-body-sm text-secondary">{label}</span>
      <span className="text-body-sm text-primary tabular-nums">{value}</span>
    </div>
  );
}
