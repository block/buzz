import { cn } from "@/shared/lib/cn";
import type { CommunityComputeKpis } from "../communityComputeMapModel";

export type CommunityComputeKpiRowProps = {
  kpis: CommunityComputeKpis;
  className?: string;
};

/** The two figures that help a member understand and grow the compute commons. */
export function CommunityComputeKpiRow({
  kpis,
  className,
}: CommunityComputeKpiRowProps) {
  const items = [
    {
      label: "People contributing",
      value: formatCount(kpis.contributorMemberCount),
      detail:
        kpis.contributorMemberCount === 0
          ? "Be the first contributor"
          : `${formatCount(kpis.memberCount)} people in this community`,
      testId: "members-sharing",
    },
    {
      label: "Compute shared",
      value: formatCapacity(kpis.sharedCapacityGb),
      detail: "Available to community agents",
      testId: "contributed-vram",
    },
  ];

  return (
    <dl
      className={cn(
        "grid grid-cols-1 overflow-hidden rounded-xl border border-border/70 bg-muted/15 sm:grid-cols-2",
        className,
      )}
      data-testid="community-compute-kpis"
    >
      {items.map((item, index) => (
        <div
          className={cn(
            "min-w-0 px-3 py-2.5",
            index > 0 && "border-border/60 border-t sm:border-l sm:border-t-0",
          )}
          data-testid={`community-compute-kpi-${item.testId}`}
          key={item.label}
        >
          <dt className="truncate text-2xs font-medium tracking-wide text-muted-foreground uppercase">
            {item.label}
          </dt>
          <dd className="mt-1 truncate text-xl font-semibold tabular-nums">
            {item.value}
          </dd>
          <dd className="mt-0.5 truncate text-2xs text-muted-foreground">
            {item.detail}
          </dd>
        </div>
      ))}
    </dl>
  );
}

function formatCount(value: number): string {
  return new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }).format(
    value,
  );
}

function formatCapacity(value: number | null): string {
  if (value === null) return "—";
  return `${new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 }).format(value)} GB`;
}
