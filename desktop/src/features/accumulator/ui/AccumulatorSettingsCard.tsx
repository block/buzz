import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { RefreshCw } from "lucide-react";
import * as React from "react";

import { runFoldsCli, runFoldsCliJson } from "@/shared/api/tauriFolds";
import { Button } from "@/shared/ui/button";
import {
  SettingsOptionGroup,
  SettingsOptionGroupList,
  SettingsOptionRow,
} from "@/features/settings/ui/SettingsOptionGroup";
import { SettingsSectionHeader } from "@/features/settings/ui/SettingsSectionHeader";

// JSON shapes printed by the `buzz folds` CLI (crates/buzz-cli/commands/folds.rs).
type FoldSummary = {
  fold: string;
  selection: string;
  schema: string;
  model: string;
  latest_version: number | null;
  covered_signals: number;
  coverage_until: number | null;
};

type FoldPlan = {
  fold: string;
  action: "cached" | "stalled" | "ready" | "run";
  note?: string;
  reason?: string;
  version?: number;
  pending_signals?: number;
  would_show?: number;
  truncated_to_budget?: boolean;
  est_input_tokens?: number;
  est_cost_usd?: number | null;
  next_version?: number;
  shown_signals?: number;
  coverage_until?: number | null;
  // Ready plans carry the exact half-open window the estimate priced, so Run
  // can replay the same fetch instead of racing new arrivals.
  since?: number;
  until_exclusive?: number;
};

const foldsListQueryKey = ["accumulator", "folds"] as const;

function formatTimestamp(seconds: number | null | undefined): string {
  if (!seconds) return "never";
  return new Date(seconds * 1000).toLocaleString();
}

function formatCost(costUsd: number | null | undefined): string {
  if (costUsd === null || costUsd === undefined) return "unknown cost";
  return `~$${costUsd.toFixed(4)}`;
}

function planSummary(plan: FoldPlan): string {
  switch (plan.action) {
    case "cached":
      return `Up to date — v${plan.version ?? "?"} already covers everything new.`;
    case "stalled":
      return `Stalled: ${plan.reason ?? "cannot fit the pending events"} (${plan.pending_signals ?? 0} pending).`;
    case "ready":
      return `${plan.pending_signals ?? 0} new signal(s); would fold ${plan.would_show ?? 0} into v${plan.next_version ?? "?"} for ${formatCost(plan.est_cost_usd)} (${plan.est_input_tokens ?? "?"} input tokens${plan.truncated_to_budget ? ", chunked to fit the context budget" : ""}).`;
    case "run":
      return `Folded ${plan.shown_signals ?? 0} signal(s) into v${plan.version ?? "?"} (coverage through ${formatTimestamp(plan.coverage_until)}).`;
  }
}

function FoldRow({ fold }: { fold: FoldSummary }) {
  const queryClient = useQueryClient();
  const [plan, setPlan] = React.useState<FoldPlan | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [showArtifact, setShowArtifact] = React.useState(false);

  const artifactQuery = useQuery({
    queryKey: ["accumulator", "artifact", fold.fold],
    queryFn: () => runFoldsCli(["artifact", fold.fold, "--raw"]),
    enabled: showArtifact,
  });

  const estimateMutation = useMutation({
    mutationFn: () => runFoldsCliJson<FoldPlan>(["estimate", fold.fold]),
    onSuccess: (result) => {
      setPlan(result);
      setError(null);
    },
    onError: (mutationError: Error) => setError(mutationError.message),
  });

  const runMutation = useMutation({
    // Replay exactly the window the preflight priced — arrivals since then
    // stay pending for the next preflight instead of silently joining a run
    // priced without them.
    mutationFn: (priced: FoldPlan) =>
      runFoldsCliJson<FoldPlan>([
        "run",
        fold.fold,
        ...(priced.since !== undefined
          ? ["--since", String(priced.since)]
          : []),
        ...(priced.until_exclusive !== undefined
          ? ["--until", String(priced.until_exclusive)]
          : []),
      ]),
    onSuccess: async (result) => {
      setPlan(result);
      setError(null);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: foldsListQueryKey }),
        queryClient.invalidateQueries({
          queryKey: ["accumulator", "artifact", fold.fold],
        }),
      ]);
    },
    onError: (mutationError: Error) => {
      // A failed run invalidates the priced plan: Run re-locks until the
      // next preflight rather than offering a stale "ready".
      setPlan(null);
      setError(mutationError.message);
    },
  });

  const busy = estimateMutation.isPending || runMutation.isPending;
  // Priced-before-spend: Run only unlocks after a preflight priced this
  // exact pending set. Anything that changes the set re-requires preflight.
  const runReady = plan?.action === "ready" && !busy;

  return (
    <SettingsOptionRow data-testid={`accumulator-fold-${fold.fold}`}>
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium">{fold.fold}</p>
        <p
          className="text-sm font-normal text-muted-foreground/70"
          data-settings-subcopy
        >
          {`${fold.selection} · ${fold.schema} · ${fold.model} · ${
            fold.latest_version
              ? `v${fold.latest_version}, covered through ${formatTimestamp(fold.coverage_until)}`
              : "never run"
          }`}
        </p>
        {plan ? (
          <p className="mt-1 text-sm text-muted-foreground">
            {planSummary(plan)}
          </p>
        ) : null}
        {error ? (
          <p className="mt-1 text-sm text-destructive">{error}</p>
        ) : null}
        {showArtifact ? (
          <pre className="mt-2 max-h-80 overflow-auto whitespace-pre-wrap rounded-md border border-border/50 bg-muted/30 p-3 text-xs">
            {artifactQuery.isPending
              ? "Loading artifact…"
              : artifactQuery.isError
                ? String(artifactQuery.error)
                : artifactQuery.data.exitCode === 0
                  ? artifactQuery.data.stdout
                  : artifactQuery.data.stderr.trim() ||
                    "This fold has no artifact yet."}
          </pre>
        ) : null}
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <Button
          data-testid={`accumulator-artifact-${fold.fold}`}
          onClick={() => setShowArtifact((visible) => !visible)}
          size="sm"
          variant="ghost"
        >
          {showArtifact ? "Hide digest" : "View digest"}
        </Button>
        <Button
          data-testid={`accumulator-estimate-${fold.fold}`}
          disabled={busy}
          onClick={() => estimateMutation.mutate()}
          size="sm"
          variant="outline"
        >
          {estimateMutation.isPending ? "Pricing…" : "Preflight"}
        </Button>
        <Button
          data-testid={`accumulator-run-${fold.fold}`}
          disabled={!runReady}
          onClick={() => plan && runMutation.mutate(plan)}
          size="sm"
        >
          {runMutation.isPending
            ? "Folding…"
            : plan?.action === "ready"
              ? `Run ${formatCost(plan.est_cost_usd)}`
              : "Run"}
        </Button>
      </div>
    </SettingsOptionRow>
  );
}

export function AccumulatorSettingsCard() {
  const foldsQuery = useQuery({
    queryKey: foldsListQueryKey,
    queryFn: () => runFoldsCliJson<FoldSummary[]>(["list"]),
  });

  return (
    <section className="min-w-0" data-testid="settings-accumulator">
      <SettingsSectionHeader
        action={
          <Button
            data-testid="accumulator-refresh"
            disabled={foldsQuery.isFetching}
            onClick={() => void foldsQuery.refetch()}
            size="sm"
            variant="outline"
          >
            <RefreshCw
              className={foldsQuery.isFetching ? "animate-spin" : undefined}
            />
            Refresh
          </Button>
        }
        title="Accumulator"
        description="Standing digests over saved relay selections. Each fold keeps one always-current artifact; preflight prices a run exactly before anything is spent."
      />
      <SettingsOptionGroupList>
        <SettingsOptionGroup title="Folds">
          {foldsQuery.isPending ? (
            <SettingsOptionRow>
              <p className="text-sm text-muted-foreground">Loading folds…</p>
            </SettingsOptionRow>
          ) : foldsQuery.isError ? (
            <SettingsOptionRow>
              <p className="text-sm text-destructive">
                {String(foldsQuery.error)}
              </p>
            </SettingsOptionRow>
          ) : foldsQuery.data.length === 0 ? (
            <SettingsOptionRow data-testid="accumulator-empty">
              <div className="min-w-0 flex-1">
                <p className="text-sm font-medium">No folds yet</p>
                <p
                  className="text-sm font-normal text-muted-foreground/70"
                  data-settings-subcopy
                >
                  Create one from a terminal, then manage it here:
                </p>
                <pre className="mt-2 overflow-x-auto rounded-md border border-border/50 bg-muted/30 p-3 text-xs">
                  {"buzz folds set my-channel-digest --channel <channel-uuid>"}
                </pre>
              </div>
            </SettingsOptionRow>
          ) : (
            foldsQuery.data.map((fold) => (
              <FoldRow fold={fold} key={fold.fold} />
            ))
          )}
        </SettingsOptionGroup>
      </SettingsOptionGroupList>
    </section>
  );
}
