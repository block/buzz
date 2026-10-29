import * as React from "react";

import {
  filterLandscape,
  formatContext,
  type LandscapeFilter,
  type LandscapeModel,
  normalizeModelId,
} from "@/features/agents/lib/modelDashboard";
import type { AgentUsageSeries } from "@/shared/api/tauriArchive";
import { Badge } from "@/shared/ui/badge";
import { Input } from "@/shared/ui/input";
import { SegmentedControl } from "@/shared/ui/segmented-control";
import { PanelMessage } from "./ModelUsagePanel";

const FILTER_OPTIONS = [
  { value: "all", label: "All" },
  { value: "frontier", label: "Frontier" },
  { value: "open", label: "Open" },
  { value: "in-use", label: "In use" },
] as const;

const VISIBLE_ROW_LIMIT = 60;

function formatPrice(value: number | null): string {
  if (value === null) return "—";
  if (value === 0) return "Free";
  return `$${Number.isInteger(value) ? value : value.toFixed(2)}`;
}

type ModelLandscapePanelProps = {
  error: Error | null;
  isLoading: boolean;
  models: LandscapeModel[];
  series: AgentUsageSeries | undefined;
};

export function ModelLandscapePanel({
  error,
  isLoading,
  models,
  series,
}: ModelLandscapePanelProps) {
  const [filter, setFilter] = React.useState<LandscapeFilter>("all");
  const [query, setQuery] = React.useState("");

  const inUse = React.useMemo(() => {
    const ids = new Set<string>();
    for (const agent of series?.agents ?? []) {
      for (const entry of agent.models) {
        if (entry.model) ids.add(normalizeModelId(entry.model));
      }
    }
    return ids;
  }, [series]);

  const visible = React.useMemo(
    () => filterLandscape(models, { filter, query, inUse }),
    [models, filter, query, inUse],
  );

  if (error) {
    return (
      <PanelMessage>Couldn't load models.dev: {error.message}</PanelMessage>
    );
  }
  if (isLoading) {
    return <PanelMessage>Loading model landscape…</PanelMessage>;
  }

  return (
    <div className="space-y-4" data-testid="agents-models-landscape">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <Input
          aria-label="Search models"
          className="h-8 max-w-xs"
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Search models or labs"
          value={query}
        />
        <SegmentedControl
          legend="Landscape filter"
          onValueChange={setFilter}
          optionTestIdPrefix="agents-models-landscape-filter"
          options={FILTER_OPTIONS}
          size="wide"
          testId="agents-models-landscape-filter"
          value={filter}
        />
      </div>

      {visible.length === 0 ? (
        <PanelMessage>
          {filter === "in-use"
            ? "None of the models your agents used match the catalog."
            : "No models match."}
        </PanelMessage>
      ) : (
        <div className="overflow-x-auto rounded-xl border border-border/70">
          <table className="w-full border-collapse text-sm">
            <thead className="bg-muted/40 text-left text-xs text-muted-foreground">
              <tr>
                <th className="px-3 py-2 font-medium">Model</th>
                <th className="px-3 py-2 font-medium">Weights</th>
                <th className="px-3 py-2 text-right font-medium">Context</th>
                <th className="px-3 py-2 text-right font-medium">$/M in</th>
                <th className="px-3 py-2 text-right font-medium">$/M out</th>
                <th className="px-3 py-2 text-right font-medium">Released</th>
              </tr>
            </thead>
            <tbody>
              {visible.slice(0, VISIBLE_ROW_LIMIT).map((model) => (
                <tr
                  className="border-t border-border/50"
                  data-testid="agents-models-landscape-row"
                  key={`${model.lab}:${model.id}`}
                >
                  <td className="px-3 py-2">
                    <div className="flex items-center gap-2 font-medium">
                      {model.name}
                      {inUse.has(normalizeModelId(model.id)) ? (
                        <Badge variant="success">In use</Badge>
                      ) : null}
                    </div>
                    <div className="text-xs text-muted-foreground">
                      {model.lab}
                      {model.reasoning ? " · reasoning" : ""}
                      {model.toolCall ? " · tools" : ""}
                    </div>
                  </td>
                  <td className="px-3 py-2">
                    {model.openWeights ? (
                      <Badge variant="info">Open</Badge>
                    ) : (
                      <Badge variant="secondary">Closed</Badge>
                    )}
                  </td>
                  <td className="px-3 py-2 text-right tabular-nums">
                    {formatContext(model.contextTokens)}
                  </td>
                  <td className="px-3 py-2 text-right tabular-nums">
                    {formatPrice(model.inputCostPerM)}
                  </td>
                  <td className="px-3 py-2 text-right tabular-nums">
                    {formatPrice(model.outputCostPerM)}
                  </td>
                  <td className="px-3 py-2 text-right tabular-nums text-muted-foreground">
                    {model.releaseDate ?? "—"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <p className="text-xs text-muted-foreground">
        {visible.length > VISIBLE_ROW_LIMIT
          ? `Showing ${VISIBLE_ROW_LIMIT} of ${visible.length}. Search to narrow. `
          : ""}
        Lab list prices and limits from{" "}
        <a
          className="underline underline-offset-2"
          href="https://models.dev"
          rel="noreferrer"
          target="_blank"
        >
          models.dev
        </a>
        . Local runs cost only your hardware.
      </p>
    </div>
  );
}
