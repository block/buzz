import type { ReactNode } from "react";

/**
 * Presentation pieces for the design system pages only. These are documentation
 * furniture, not product components — the shared primitive layer gets built one
 * component at a time as the product repeats something.
 *
 * Surface convention on these pages: a region is separated by a soft fill, not
 * by an outline. Reach for `bg-inset` before reaching for a border; use a
 * hairline only where a genuine boundary is needed, and never above
 * `border-secondary`. See DESIGN.md § Surface and depth.
 */

export function PageHeader({
  title,
  intro,
  status,
}: {
  title: string;
  intro: string;
  status?: string;
}) {
  return (
    <header className="mb-14 flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="text-title text-primary">{title}</h1>
        {status ? <StatusPill>{status}</StatusPill> : null}
      </div>
      <p className="max-w-2xl text-body-lg text-secondary">{intro}</p>
    </header>
  );
}

export function StatusPill({ children }: { children: ReactNode }) {
  return (
    <span className="rounded-full bg-warning-tint px-2.5 py-1 text-body-sm text-warning">
      {children}
    </span>
  );
}

export function Section({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <section className="mb-14 flex flex-col gap-5">
      <div className="flex flex-col gap-2">
        <h2 className="text-heading text-primary">{title}</h2>
        {description ? (
          <p className="max-w-2xl text-body text-secondary">{description}</p>
        ) : null}
      </div>
      {children}
    </section>
  );
}

export function Note({ children }: { children: ReactNode }) {
  return (
    <p className="max-w-2xl rounded-xl bg-inset px-5 py-4 text-body-sm text-secondary">
      {children}
    </p>
  );
}

export function Stub({ what, decide }: { what: string; decide: string[] }) {
  return (
    <div className="flex max-w-2xl flex-col gap-4 rounded-xl bg-inset px-6 py-5">
      <p className="text-body text-secondary">{what}</p>
      <div className="flex flex-col gap-2">
        <p className="text-body text-tertiary">Still to decide</p>
        <ul className="flex list-disc flex-col gap-1.5 pl-4">
          {decide.map((item) => (
            <li key={item} className="text-body-sm text-secondary">
              {item}
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}

/**
 * A list of uniform rows — the tabular case, where every row looks alike and the
 * eye needs a line to track along. Dividers, no container: the section heading
 * already says these belong together, so a fill behind them adds a box without
 * adding meaning. See DESIGN.md § Density and rhythm.
 *
 * If the rows carry their own visual difference — a swatch, a type specimen — use
 * `Specimens` instead. Content that separates itself needs no divider.
 */
export function Rows({ children }: { children: ReactNode }) {
  return <div className="flex flex-col">{children}</div>;
}

export function Row({ children }: { children: ReactNode }) {
  return (
    <div className="border-tertiary border-b py-3 last:border-b-0">
      {children}
    </div>
  );
}

/**
 * A list whose entries are visibly different from each other — colour swatches,
 * type specimens, elevation samples.
 *
 * No container and no dividers. The specimen is its own separator, and two
 * things a colour system must never do are judge a swatch against a fill it
 * will never sit on, or set a type specimen in a box that changes its contrast.
 * Separation comes from space alone.
 */
export function Specimens({ children }: { children: ReactNode }) {
  return <div className="flex flex-col gap-7">{children}</div>;
}

/** Renders a live swatch of whatever a CSS custom property currently holds. */
export function Swatch({
  variable,
  label,
  sublabel,
  translucent,
}: {
  variable: string;
  label: string;
  sublabel?: string;
  translucent?: boolean;
}) {
  return (
    <div className="flex min-w-0 flex-col gap-2">
      <div
        className={`h-16 rounded-lg border-tertiary border ${
          translucent ? "blur-chrome" : ""
        }`}
        style={{ background: `var(${variable})` }}
      />
      <code className="truncate text-mono text-primary">{label}</code>
      {sublabel ? (
        <span className="text-body-sm text-tertiary">{sublabel}</span>
      ) : null}
    </div>
  );
}
