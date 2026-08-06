import { Medal, Trophy } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import type { useMeshComputeState } from "../hooks/useMeshComputeState";

const PREVIEW_LEADERS = [
  { name: "Maya", tokens: 2_840_912 },
  { name: "Jon", tokens: 1_967_401 },
  { name: "Priya", tokens: 1_204_885 },
] as const;

export function MeshComputeTokenLeaderboard({
  mesh,
  simulated = false,
}: {
  mesh: ReturnType<typeof useMeshComputeState>;
  simulated?: boolean;
}) {
  const yourTokens = simulated ? 1_638_240 : (mesh.usage?.tokensServed ?? 0);
  const communityLeaders = simulated ? PREVIEW_LEADERS : [];
  const rows = [...communityLeaders, { name: "You", tokens: yourTokens }]
    .sort((a, b) => b.tokens - a.tokens)
    .map((entry, index) => ({ ...entry, rank: index + 1 }));
  const yourRank =
    rows.find((entry) => entry.name === "You")?.rank ?? rows.length;

  return (
    <section
      className="overflow-hidden rounded-2xl border border-border/70 bg-background"
      data-testid="compute-token-leaderboard"
    >
      <div className="flex items-start gap-3 border-border/60 border-b px-4 py-3.5">
        <span className="flex size-9 shrink-0 items-center justify-center rounded-xl bg-amber-500/15 text-amber-700 dark:text-amber-300">
          <Trophy className="size-4" />
        </span>
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold">
            Community token contributors
          </h2>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {simulated
              ? "Tokenmaxxing, inverted: celebrate the tokens you serve—not the tokens you consume."
              : "Your locally reported total. Community rankings will appear when aggregate reporting is available."}
          </p>
        </div>
        {simulated ? (
          <div className="shrink-0 text-right">
            <p className="text-2xs font-medium uppercase tracking-wide text-muted-foreground">
              Your rank
            </p>
            <p className="mt-0.5 text-lg font-semibold tabular-nums">
              #{yourRank}
            </p>
          </div>
        ) : null}
      </div>

      <ol className="divide-y divide-border/60">
        {rows.map((entry) => (
          <li
            className={cn(
              "grid grid-cols-[2.5rem_1fr_auto] items-center gap-2 px-4 py-2.5",
              entry.name === "You" && "bg-foreground/[0.045]",
            )}
            key={entry.name}
          >
            <span className="flex items-center gap-1 text-xs font-semibold tabular-nums text-muted-foreground">
              {entry.rank <= 3 ? <Medal className="size-3.5" /> : null}#
              {entry.rank}
            </span>
            <span className="truncate text-sm font-medium">
              {entry.name}
              {entry.name === "You" ? (
                <span className="ml-2 rounded-full bg-foreground px-2 py-0.5 text-3xs font-semibold text-background">
                  YOU
                </span>
              ) : null}
            </span>
            <span className="text-sm font-semibold tabular-nums">
              {formatTokens(entry.tokens)}
              <span className="ml-1 text-2xs font-normal text-muted-foreground">
                tokens
              </span>
            </span>
          </li>
        ))}
      </ol>

      <p className="border-border/60 border-t bg-muted/10 px-4 py-2 text-2xs text-muted-foreground">
        {simulated
          ? "Tutorial preview · example contribution totals"
          : "Community rankings preview · aggregate contribution reporting is coming soon."}
      </p>
    </section>
  );
}

function formatTokens(value: number): string {
  return new Intl.NumberFormat(undefined, {
    notation: value >= 10_000 ? "compact" : "standard",
    maximumFractionDigits: 1,
  }).format(value);
}
