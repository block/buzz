import { AdviserInsignia, type CommandAdviserId } from "./AdviserInsignia";

const COMMAND_TEAM: readonly [CommandAdviserId, string, string][] = [
  ["chief_of_staff", "Chief of Staff", "Consolidates the command brief"],
  ["operations", "Operations", "Priorities, readiness and risk"],
  ["navigation", "Navigation", "Evidence and source limitations"],
  ["daily_routine", "Daily Routine", "Calendar, reminders and routine"],
  ["reporting", "Reporting", "Reports, returns and missing inputs"],
  ["plans", "Plans", "30, 60 and 90-day outlook"],
];

export function CommandTeamStrip() {
  return (
    <section
      aria-labelledby="command-team-heading"
      className="space-y-3"
      data-testid="command-team"
    >
      <div>
        <p className="text-xs font-semibold uppercase tracking-widest text-[#d8aa4f]">
          Virtual command team
        </p>
        <h2 className="mt-1 text-lg font-semibold" id="command-team-heading">
          Adviser team
        </h2>
      </div>
      <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
        {COMMAND_TEAM.map(([id, label, detail]) => (
          <div
            className="flex items-center gap-3 rounded-xl border border-border/70 bg-card/80 p-3 shadow-xs"
            key={id}
          >
            <AdviserInsignia adviser={id} />
            <div className="min-w-0">
              <h3 className="text-sm font-semibold">{label}</h3>
              <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">
                {detail}
              </p>
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}
