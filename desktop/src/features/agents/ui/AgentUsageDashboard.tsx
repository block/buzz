import { RefreshCw } from "lucide-react";

import { useAgentUsageDashboardQuery } from "@/features/agents/hooks";
import type { AgentUsageSummary } from "@/shared/api/agentUsageTypes";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/shared/ui/card";

const COMPACT_NUMBER = new Intl.NumberFormat("en-US", {
  maximumFractionDigits: 1,
  notation: "compact",
});

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function formatPromptEstimate(summary: AgentUsageSummary): string {
  if (summary.promptCount === 0) return "—";
  return `≈${COMPACT_NUMBER.format(summary.estimatedPromptTokens)}`;
}

type AgentUsageDashboardProps = {
  onOpenAgent: (pubkey: string) => void;
};

export function AgentUsageDashboard({ onOpenAgent }: AgentUsageDashboardProps) {
  const usageQuery = useAgentUsageDashboardQuery();
  const summaries = usageQuery.data ?? [];

  return (
    <Card className="mx-auto w-full max-w-[996px]">
      <CardHeader className="gap-3 sm:flex-row sm:items-start sm:justify-between sm:space-y-0">
        <div className="space-y-1.5">
          <CardTitle className="text-base">Agent usage</CardTitle>
          <CardDescription>
            Buzz-side input measurements from each local agent log. Estimated
            tokens are prompt bytes ÷ 4, not provider billing totals.
          </CardDescription>
        </div>
        <Button
          aria-label="Refresh agent usage"
          disabled={usageQuery.isFetching}
          onClick={() => {
            void usageQuery.refetch();
          }}
          size="sm"
          variant="outline"
        >
          <RefreshCw className={usageQuery.isFetching ? "animate-spin" : ""} />
          Refresh
        </Button>
      </CardHeader>
      <CardContent>
        {usageQuery.error instanceof Error ? (
          <p className="text-sm text-destructive">
            Could not read agent usage: {usageQuery.error.message}
          </p>
        ) : usageQuery.isLoading ? (
          <p className="text-sm text-muted-foreground">Loading usage…</p>
        ) : summaries.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No local agent instances are available yet.
          </p>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full min-w-[780px] border-collapse text-left text-sm">
              <thead>
                <tr className="border-b border-border/70 text-xs text-muted-foreground">
                  <th className="pb-2 pr-4 font-medium">Agent</th>
                  <th className="px-3 pb-2 font-medium">Workers</th>
                  <th className="px-3 pb-2 font-medium">Prompts</th>
                  <th className="px-3 pb-2 font-medium">Est. input</th>
                  <th className="px-3 pb-2 font-medium">Prompt bytes</th>
                  <th className="px-3 pb-2 font-medium">Peak</th>
                  <th className="px-3 pb-2 font-medium">Retries</th>
                  <th className="pl-3 pb-2 font-medium">Quota stops</th>
                </tr>
              </thead>
              <tbody>
                {summaries.map((summary) => (
                  <tr
                    className="border-b border-border/50 last:border-b-0"
                    key={summary.pubkey}
                  >
                    <td className="py-3 pr-4">
                      <button
                        className="font-medium hover:underline focus-visible:rounded-sm focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                        onClick={() => onOpenAgent(summary.pubkey)}
                        type="button"
                      >
                        {summary.name}
                      </button>
                      <div className="max-w-48 truncate text-xs text-muted-foreground">
                        {summary.model ?? "Inherited model"}
                        {summary.isRunning ? " · running" : " · stopped"}
                      </div>
                    </td>
                    <td className="px-3 py-3">
                      <Badge
                        variant={
                          summary.parallelism > 10 ? "warning" : "secondary"
                        }
                      >
                        {summary.parallelism}
                      </Badge>
                    </td>
                    <td className="px-3 py-3 tabular-nums">
                      <div>{summary.promptCount.toLocaleString()}</div>
                      <div className="text-xs text-muted-foreground">
                        {summary.sessionStartCount.toLocaleString()} sessions
                      </div>
                    </td>
                    <td className="px-3 py-3 tabular-nums">
                      {formatPromptEstimate(summary)}
                    </td>
                    <td className="px-3 py-3 tabular-nums">
                      {summary.promptCount > 0
                        ? formatBytes(summary.promptBytes)
                        : "—"}
                    </td>
                    <td className="px-3 py-3 tabular-nums">
                      <div>
                        {summary.promptCount > 0
                          ? formatBytes(summary.peakPromptBytes)
                          : "—"}
                      </div>
                      {summary.largePromptCount > 0 ? (
                        <div className="text-xs text-amber-600 dark:text-amber-400">
                          {summary.largePromptCount.toLocaleString()} over 50 KB
                        </div>
                      ) : null}
                    </td>
                    <td className="px-3 py-3 tabular-nums">
                      {summary.retryCount.toLocaleString()}
                    </td>
                    <td className="pl-3 py-3 tabular-nums">
                      {summary.quotaLimitCount.toLocaleString()}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        {summaries.length > 0 ? (
          <p className="mt-3 text-xs text-muted-foreground">
            Each row samples at most the latest 20,000 log lines. Provider
            adapters do not consistently expose exact input, output, cache, or
            billing totals.
          </p>
        ) : null}
      </CardContent>
    </Card>
  );
}
