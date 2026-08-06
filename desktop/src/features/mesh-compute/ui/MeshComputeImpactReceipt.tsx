import { Activity, Cpu, Network, Sparkles } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import type { useMeshComputeState } from "../hooks/useMeshComputeState";

export function MeshComputeImpactReceipt({
  mesh,
}: {
  mesh: ReturnType<typeof useMeshComputeState>;
}) {
  const { memory, toggle, usage } = mesh;
  const isSharing = toggle.isSharing;
  const tokensProcessed = usage?.tokensServed ?? 0;
  const peers = usage?.peers ?? 0;
  const capacity = memory.totalGb == null ? "—" : formatGb(memory.totalGb);

  const items = [
    {
      icon: Cpu,
      label: "Capacity offered",
      value: isSharing ? capacity : "Not sharing",
    },
    {
      icon: Sparkles,
      label: "Tokens served",
      value: isSharing ? formatCount(tokensProcessed) : "—",
    },
    {
      icon: Network,
      label: "Mesh peers",
      value: isSharing ? formatCount(peers) : "—",
    },
  ];

  return (
    <section
      className={cn(
        "overflow-hidden rounded-2xl border border-border/70 bg-background",
        !isSharing && "bg-muted/10",
      )}
      data-testid="compute-impact-receipt"
    >
      <div className="flex items-start gap-3 border-border/60 border-b px-4 py-3">
        <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-foreground text-background">
          <Activity className="size-4" />
        </span>
        <div>
          <h2 className="text-sm font-semibold">Your contribution</h2>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {isSharing
              ? usage?.inflight
                ? `Helping run ${usage.inflight} request${usage.inflight === 1 ? "" : "s"} right now.`
                : "Available when your community needs more compute."
              : "Turn on automatic contribution to start building your impact."}
          </p>
        </div>
        {isSharing ? (
          <span className="ml-auto flex shrink-0 items-center gap-1.5 text-xs font-medium">
            <span className="size-2 rounded-full bg-emerald-500" /> Live
          </span>
        ) : null}
      </div>
      <dl className="grid grid-cols-3 divide-x divide-border/60">
        {items.map(({ icon: Icon, label, value }) => (
          <div className="min-w-0 px-4 py-3" key={label}>
            <dt className="flex items-center gap-1.5 truncate text-2xs text-muted-foreground">
              <Icon className="size-3" /> {label}
            </dt>
            <dd className="mt-1 truncate text-lg font-semibold tabular-nums">
              {value}
            </dd>
          </div>
        ))}
      </dl>
      <p className="border-border/60 border-t px-4 py-2 text-2xs text-muted-foreground">
        Session totals from this machine. Request attribution to other members
        is coming soon.
      </p>
    </section>
  );
}

function formatCount(value: number): string {
  return new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }).format(
    value,
  );
}

function formatGb(value: number): string {
  return `${new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 }).format(value)} GB`;
}
