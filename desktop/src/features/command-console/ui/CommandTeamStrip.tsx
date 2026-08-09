import { MessageCircle } from "lucide-react";

import { usePersonaConversation } from "@/features/agents/usePersonaConversation";
import { Button } from "@/shared/ui/button";
import { COMMAND_TEAM_PERSONAS } from "../domain/commandTeam";
import { AdviserInsignia } from "./AdviserInsignia";

export function CommandTeamStrip() {
  const conversation = usePersonaConversation();
  return (
    <CommandTeamStripView
      error={conversation.error}
      onMessage={(personaId) => {
        void conversation.open(personaId);
      }}
      pendingPersonaIds={conversation.pendingPersonaIds}
    />
  );
}

export function CommandTeamStripView({
  error,
  onMessage,
  pendingPersonaIds,
}: {
  error: string | null;
  onMessage: (personaId: string) => void;
  pendingPersonaIds: ReadonlySet<string>;
}) {
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
            <div className="min-w-0 flex-1">
              <h3 className="text-sm font-semibold">{label}</h3>
              <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">
                {detail}
              </p>
            </div>
            <Button
              disabled={pendingPersonaIds.has(personaId)}
              onClick={() => onMessage(personaId)}
              size="sm"
              type="button"
              variant="secondary"
            >
              <MessageCircle />
              Message
            </Button>
          </div>
        ))}
      </div>
      {error ? (
        <p className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </p>
      ) : null}
    </section>
  );
}
