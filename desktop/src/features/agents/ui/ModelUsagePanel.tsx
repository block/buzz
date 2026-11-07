import type * as React from "react";

import {
  aggregateUsageByModel,
  displayTokens,
  formatTokensCompact,
  formatUsd,
  type LandscapeModel,
  type ModelClassification,
  type ModelOrigin,
  totalsByOrigin,
} from "@/features/agents/lib/modelDashboard";
import type { AgentUsageSeries } from "@/shared/api/tauriArchive";
import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";

const ORIGIN_LABEL: Record<ModelOrigin, string> = {
  frontier: "Frontier API",
  open: "Open-weight",
  unknown: "Unclassified",
};

const ORIGIN_BAR_CLASS: Record<ModelOrigin, string> = {
  frontier: "bg-primary",
  open: "bg-emerald-500",
  unknown: "bg-muted-foreground/40",
};

export function OriginBadge({
  classification,
}: {
  classification: ModelClassification;
}) {
  if (classification.isLocal) {
    return <Badge variant="success">Local</Badge>;
  }
  if (classification.origin === "open") {
    return <Badge variant="info">Open-weight</Badge>;
  }
  if (classification.origin === "frontier") {
    return <Badge variant="secondary">Frontier</Badge>;
  }
  return <Badge variant="outline">Unknown</Badge>;
}

type ModelUsagePanelProps = {
  catalog: ReadonlyMap<string, LandscapeModel> | undefined;
  error: Error | null;
  isLoading: boolean;
  series: AgentUsageSeries | undefined;
  windowControl: React.ReactNode;
};

export function ModelUsagePanel({
  catalog,
  error,
  isLoading,
  series,
  windowControl,
}: ModelUsagePanelProps) {
  if (error) {
    return (
      <PanelMessage>Couldn't read local usage: {error.message}</PanelMessage>
    );
  }
  if (isLoading || !series) {
    return <PanelMessage>Loading usage…</PanelMessage>;
  }
  if (!series.collectionEnabled) {
    return (
      <PanelMessage>
        Agent usage collection is off. Turn on agent metrics in Settings → Local
        archive to see per-model usage here.
      </PanelMessage>
    );
  }

  const rows = aggregateUsageByModel(series, catalog);
  const totals = totalsByOrigin(rows).filter((t) => t.modelCount > 0);
  const grandTokens = totals.reduce((sum, t) => sum + t.totalTokens, 0n);

  return (
    <div className="space-y-4" data-testid="agents-models-usage">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className="text-sm text-muted-foreground">
          {series.coverage.reportCount.toLocaleString("en-US")} agent turns ·{" "}
          {rows.length} {rows.length === 1 ? "model" : "models"}
        </p>
        {windowControl}
      </div>

      {rows.length === 0 ? (
        <PanelMessage>
          No agent turns reported usage in this window yet.
        </PanelMessage>
      ) : (
        <>
          <div className="grid gap-3 sm:grid-cols-3">
            {totals.map((total) => (
              <div
                className="rounded-xl border border-border/70 bg-card/60 p-4"
                data-testid={`agents-models-total-${total.origin}`}
                key={total.origin}
              >
                <p className="text-xs font-medium text-muted-foreground">
                  {ORIGIN_LABEL[total.origin]}
                </p>
                <p className="mt-1 text-2xl font-semibold tabular-nums">
                  {formatTokensCompact(total.totalTokens)}
                  <span className="ml-1 text-sm font-normal text-muted-foreground">
                    tokens
                  </span>
                </p>
                <p className="mt-0.5 text-sm text-muted-foreground tabular-nums">
                  {total.costUsd > 0
                    ? `${formatUsd(total.costUsd)} est.`
                    : "No reported cost"}{" "}
                  · {total.modelCount}{" "}
                  {total.modelCount === 1 ? "model" : "models"}
                </p>
              </div>
            ))}
          </div>

          {grandTokens > 0n ? (
            <div
              aria-label="Token share by model origin"
              className="flex h-2 w-full overflow-hidden rounded-full bg-muted"
              role="img"
            >
              {totals.map((total) => (
                <div
                  className={ORIGIN_BAR_CLASS[total.origin]}
                  key={total.origin}
                  style={{
                    width: `${(Number((total.totalTokens * 10_000n) / grandTokens) / 100).toFixed(2)}%`,
                  }}
                />
              ))}
            </div>
          ) : null}

          <div className="overflow-x-auto rounded-xl border border-border/70">
            <table className="w-full border-collapse text-sm">
              <thead className="bg-muted/40 text-left text-xs text-muted-foreground">
                <tr>
                  <th className="px-3 py-2 font-medium">Model</th>
                  <th className="px-3 py-2 font-medium">Type</th>
                  <th className="px-3 py-2 text-right font-medium">Tokens</th>
                  <th className="px-3 py-2 text-right font-medium">Share</th>
                  <th className="px-3 py-2 text-right font-medium">
                    Est. cost
                  </th>
                  <th className="px-3 py-2 text-right font-medium">Turns</th>
                  <th className="px-3 py-2 text-right font-medium">Agents</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((row) => {
                  const tokens = displayTokens(row);
                  const share =
                    tokens !== null && grandTokens > 0n
                      ? Number((tokens * 1_000n) / grandTokens) / 10
                      : null;
                  return (
                    <tr
                      className="border-t border-border/50"
                      data-testid="agents-models-usage-row"
                      key={row.key}
                    >
                      <td className="px-3 py-2">
                        <div className="font-medium">
                          {row.model ?? "Unreported model"}
                        </div>
                        {row.harnesses.length > 0 ? (
                          <div className="text-xs text-muted-foreground">
                            {row.harnesses.join(", ")}
                          </div>
                        ) : null}
                      </td>
                      <td className="px-3 py-2">
                        <OriginBadge classification={row.classification} />
                      </td>
                      <td
                        className={cn(
                          "px-3 py-2 text-right tabular-nums",
                          row.incomplete && "text-muted-foreground",
                        )}
                        title={
                          row.incomplete
                            ? "Some turns didn't report token counts"
                            : undefined
                        }
                      >
                        {tokens === null ? "—" : formatTokensCompact(tokens)}
                        {row.incomplete && tokens !== null ? "+" : ""}
                      </td>
                      <td className="px-3 py-2 text-right tabular-nums text-muted-foreground">
                        {share === null ? "—" : `${share}%`}
                      </td>
                      <td className="px-3 py-2 text-right tabular-nums">
                        {row.costUsd === null ? "—" : formatUsd(row.costUsd)}
                      </td>
                      <td className="px-3 py-2 text-right tabular-nums">
                        {row.reportCount.toLocaleString("en-US")}
                      </td>
                      <td className="px-3 py-2 text-right tabular-nums">
                        {row.agentCount}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </>
      )}
    </div>
  );
}

export function PanelMessage({ children }: { children: React.ReactNode }) {
  return (
    <div className="rounded-xl border border-dashed border-border/70 px-4 py-6 text-center text-sm text-muted-foreground">
      {children}
    </div>
  );
}
