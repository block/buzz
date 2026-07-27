import { COMMAND_TEAM_PERSONAS } from "../domain/commandTeam";
import { AdviserInsignia } from "./AdviserInsignia";

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
        {COMMAND_TEAM_PERSONAS.map(({ adviser, detail, label, personaId }) => (
          <div
            className="flex items-center gap-3 rounded-xl border border-border/70 bg-card/80 p-3 shadow-xs"
            data-persona-id={personaId}
            key={personaId}
          >
            <AdviserInsignia adviser={adviser} />
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
