import * as React from "react";

import type {
  SurfaceNode,
  SurfaceSpec,
  SurfaceTone,
} from "@/features/surfaces/spec";
import { formatScalar, isNumeric } from "@/features/surfaces/spec";
import { cn } from "@/shared/lib/cn";
import { Progress } from "@/shared/ui/progress";
import { useSmoothCorners } from "@/shared/ui/smoothCorners";

// Pure presentational renderer for a parsed SurfaceSpec. All content is
// data-only plain text — no markdown, no links, no media. Tone is expressed
// with color plus a distinct per-tone glyph (never color alone).

const badgeToneClass: Record<SurfaceTone, string> = {
  default: "bg-muted text-muted-foreground",
  success: "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400",
  warning: "bg-amber-500/15 text-amber-600 dark:text-amber-400",
  danger: "bg-red-500/15 text-red-600 dark:text-red-400",
  info: "bg-sky-500/15 text-sky-600 dark:text-sky-400",
};

const textToneClass: Record<SurfaceTone, string> = {
  default: "text-foreground",
  success: "text-emerald-600 dark:text-emerald-400",
  warning: "text-amber-600 dark:text-amber-400",
  danger: "text-red-600 dark:text-red-400",
  info: "text-sky-600 dark:text-sky-400",
};

// Distinct per-tone glyphs so tone survives without color perception
// (WCAG use-of-color): success check, warning bang, danger cross, info i.
const toneGlyph: Record<SurfaceTone, string | null> = {
  default: null,
  success: "✓",
  warning: "!",
  danger: "✕",
  info: "i",
};

function ToneGlyph({ tone }: { tone: SurfaceTone }) {
  const glyph = toneGlyph[tone];
  if (glyph === null) {
    return null;
  }
  return (
    <>
      <span aria-hidden className="font-semibold">
        {glyph}
      </span>
      <span className="sr-only">({tone})</span>
    </>
  );
}

function ToneBadge({ text, tone }: { text: string; tone: SurfaceTone }) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-md px-2 py-0.5 text-2xs font-semibold uppercase tracking-wide",
        badgeToneClass[tone],
      )}
      data-testid="surface-badge"
      data-tone={tone}
    >
      <ToneGlyph tone={tone} />
      {text}
    </span>
  );
}

function NodeView({ node }: { node: SurfaceNode }) {
  switch (node.type) {
    case "heading":
      return (
        <h4
          className="text-sm font-semibold text-foreground"
          data-testid="surface-heading"
        >
          {node.text}
        </h4>
      );
    case "text":
      return (
        <p
          className="whitespace-pre-wrap break-words text-sm text-foreground/90"
          data-testid="surface-text"
        >
          {node.text}
        </p>
      );
    case "badge":
      return <ToneBadge text={node.text} tone={node.tone} />;
    case "keyValue":
      return (
        <dl
          className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1"
          data-testid="surface-keyvalue"
        >
          {node.items.map((item, i) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: spec is an immutable document — an edit replaces the whole spec, labels may duplicate
            <React.Fragment key={i}>
              <dt className="text-sm text-muted-foreground">{item.label}</dt>
              <dd
                className={cn(
                  "inline-flex items-baseline gap-1 text-sm font-medium",
                  isNumeric(item.value) && "tabular-nums",
                  textToneClass[item.tone],
                )}
              >
                <ToneGlyph tone={item.tone} />
                {formatScalar(item.value)}
              </dd>
            </React.Fragment>
          ))}
        </dl>
      );
    case "statGrid":
      return (
        <div
          className="grid grid-cols-2 gap-2 sm:grid-cols-3"
          data-testid="surface-statgrid"
        >
          {node.stats.map((stat, i) => (
            <div
              className="rounded-lg border border-border/60 bg-muted/30 px-3 py-2"
              // biome-ignore lint/suspicious/noArrayIndexKey: spec is an immutable document — labels may duplicate
              key={i}
            >
              <div className="text-2xs font-medium uppercase tracking-wide text-muted-foreground">
                {stat.label}
              </div>
              <div
                className={cn(
                  "flex items-baseline gap-1 text-sm font-semibold tabular-nums",
                  textToneClass[stat.tone],
                )}
              >
                <ToneGlyph tone={stat.tone} />
                {formatScalar(stat.value)}
              </div>
              {stat.delta !== undefined && (
                <div className="text-2xs tabular-nums text-muted-foreground">
                  {formatScalar(stat.delta)}
                </div>
              )}
            </div>
          ))}
        </div>
      );
    case "table":
      return (
        <div
          className="overflow-x-auto rounded-lg border border-border/60"
          data-testid="surface-table"
        >
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-border/60 bg-muted/40">
                {node.columns.map((col, i) => (
                  <th
                    className="px-3 py-1.5 text-left font-medium text-muted-foreground"
                    // biome-ignore lint/suspicious/noArrayIndexKey: spec is an immutable document — column names may duplicate
                    key={i}
                    scope="col"
                  >
                    {col}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {node.rows.map((row, r) => (
                <tr
                  className="border-b border-border/40 last:border-b-0"
                  // biome-ignore lint/suspicious/noArrayIndexKey: spec is an immutable document — rows have no ids
                  key={r}
                >
                  {row.map((cell, c) => (
                    <td
                      className={cn(
                        "px-3 py-1.5 text-foreground/90",
                        isNumeric(cell) && "tabular-nums",
                      )}
                      // biome-ignore lint/suspicious/noArrayIndexKey: spec is an immutable document — cells are positional
                      key={c}
                    >
                      {formatScalar(cell)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    case "progress":
      return (
        <div data-testid="surface-progress">
          <div className="mb-1 flex items-baseline justify-between gap-2">
            {node.label !== undefined && (
              <span className="text-sm text-foreground/90">{node.label}</span>
            )}
            <span className="text-sm tabular-nums text-muted-foreground">
              {Math.round(node.value)}%
            </span>
          </div>
          <Progress aria-label={node.label ?? "Progress"} value={node.value} />
        </div>
      );
  }
}

export default function SurfaceCard({ spec }: { spec: SurfaceSpec }) {
  const cardRef = React.useRef<HTMLDivElement | null>(null);
  useSmoothCorners(cardRef);

  // Consecutive badges flow onto one row, matching how authors use them.
  const groups: SurfaceNode[][] = [];
  for (const node of spec.nodes) {
    const last = groups[groups.length - 1];
    if (node.type === "badge" && last?.[0]?.type === "badge") {
      last.push(node);
    } else {
      groups.push([node]);
    }
  }

  return (
    <div
      className="max-w-xl overflow-hidden rounded-2xl border border-border/70 bg-card/60"
      data-testid="surface-card"
      ref={cardRef}
    >
      <div className="flex flex-col gap-3 px-4 py-3">
        {spec.title !== undefined && (
          <div className="text-2xs font-semibold uppercase tracking-wider text-muted-foreground">
            {spec.title}
          </div>
        )}
        {groups.map((group, i) =>
          group.length > 1 ? (
            // biome-ignore lint/suspicious/noArrayIndexKey: spec is an immutable document — node groups are positional
            <div className="flex flex-wrap items-center gap-1.5" key={i}>
              {group.map((node, j) => (
                // biome-ignore lint/suspicious/noArrayIndexKey: spec is an immutable document — badges are positional
                <NodeView key={j} node={node} />
              ))}
            </div>
          ) : (
            // biome-ignore lint/suspicious/noArrayIndexKey: spec is an immutable document — node groups are positional
            <NodeView key={i} node={group[0]} />
          ),
        )}
      </div>
    </div>
  );
}
