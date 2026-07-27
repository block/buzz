import type { ReactNode } from "react";

import hmasSupplyBadge from "@/assets/command-adviser/hmas-supply-badge.png";
import hmasSupplyPhoto from "@/assets/command-adviser/hmas-supply.jpg";

export function CommandAdviserHero({
  routingControls,
}: {
  routingControls: ReactNode;
}) {
  return (
    <section
      className="overflow-hidden rounded-2xl border border-[#d8aa4f]/25 bg-[#06172b] text-white shadow-xl"
      data-testid="command-console-official-banner"
    >
      <div className="grid min-h-64 lg:grid-cols-[minmax(0,1.05fr)_minmax(20rem,0.95fr)]">
        <div className="flex items-center gap-5 p-6 lg:p-8">
          <img
            alt="HMAS Supply badge"
            className="h-28 w-auto shrink-0 object-contain drop-shadow-lg"
            src={hmasSupplyBadge}
          />
          <div className="min-w-0">
            <div className="mb-4 flex flex-wrap items-center gap-2">
              <span className="rounded-full border border-[#e4bb62]/50 bg-[#e4bb62]/10 px-3 py-1 text-2xs font-semibold uppercase tracking-widest text-[#f0cc7d]">
                OFFICIAL
              </span>
              <span className="text-xs uppercase tracking-widest text-slate-300">
                Virtual command team
              </span>
            </div>
            <p className="text-sm font-semibold uppercase tracking-widest text-[#e4bb62]">
              COMMAND ADVISER
            </p>
            <h1 className="mt-2 text-3xl font-semibold tracking-tight">
              HMAS SUPPLY · A195
            </h1>
            <p className="mt-3 text-sm font-medium uppercase tracking-widest text-slate-300">
              STRENGTHEN THE SHIELD
            </p>
            <p className="mt-5 max-w-xl text-sm leading-relaxed text-slate-300">
              Evidence-backed awareness, decisions and forward planning for
              today and the horizon ahead.
            </p>
          </div>
        </div>
        <div className="relative min-h-56 border-t border-white/10 lg:min-h-64 lg:border-l lg:border-t-0">
          <img
            alt="HMAS Supply at sea"
            className="absolute inset-0 h-full w-full object-cover object-center"
            src={hmasSupplyPhoto}
          />
          <div
            aria-hidden="true"
            className="absolute inset-0 bg-[#03101f]/25"
          />
        </div>
      </div>
      <div className="border-t border-white/10 bg-[#041225] p-4 sm:p-5">
        {routingControls}
      </div>
    </section>
  );
}
