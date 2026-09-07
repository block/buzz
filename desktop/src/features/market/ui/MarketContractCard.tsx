import { Bot, PackageCheck, Radio, Store } from "lucide-react";

import type { MarketScenario } from "@/features/market/lib/marketPrototypeData";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";

const COMMERCIAL_TERM_LABELS = new Set(["Price", "Reward", "Initial quantity"]);

export function MarketContractCard({ scenario }: { scenario: MarketScenario }) {
  const commercial = scenario.terms.find((term) =>
    COMMERCIAL_TERM_LABELS.has(term.label),
  );
  const author =
    scenario.terms.find(
      (term) => term.label === "Seller" || term.label === "Requester",
    )?.value ?? "Market agent";

  return (
    <section
      className="mt-4 overflow-hidden rounded-2xl border bg-card"
      data-testid="market-contract-card"
    >
      <div className="grid sm:grid-cols-[11rem_minmax(0,1fr)]">
        <div className="m-4 flex aspect-square items-center justify-center overflow-hidden rounded-xl bg-muted/60 text-muted-foreground sm:mr-0">
          {scenario.imageUrl ? (
            <img
              alt={scenario.title}
              className="h-full w-full object-cover"
              data-testid="market-contract-image"
              src={rewriteRelayUrl(scenario.imageUrl)}
            />
          ) : (
            <PackageCheck className="h-8 w-8" strokeWidth={1.5} />
          )}
        </div>
        <div className="min-w-0 p-4">
          <h2 className="text-xl font-semibold tracking-tight">
            {scenario.title}
          </h2>
          <p className="mt-1 text-sm leading-relaxed text-muted-foreground">
            {scenario.summary}
          </p>
          <div className="mt-4 grid gap-3 border-t pt-4 sm:grid-cols-3">
            <ContractDatum icon={Bot} label="Created by" value={author} />
            <ContractDatum
              icon={Store}
              label={commercial?.label ?? "Terms"}
              value={commercial?.value ?? scenario.direction}
            />
            <ContractDatum
              icon={Radio}
              label="Pulse discovery"
              value="Announced · channel authoritative"
            />
          </div>
        </div>
      </div>
    </section>
  );
}

function ContractDatum({
  icon: Icon,
  label,
  value,
}: {
  icon: typeof Store;
  label: string;
  value: string;
}) {
  return (
    <div className="min-w-0">
      <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
        <Icon className="h-3.5 w-3.5" /> {label}
      </p>
      <p className="mt-1 truncate text-sm font-medium" title={value}>
        {value}
      </p>
    </div>
  );
}
