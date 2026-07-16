import * as React from "react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { listPersonas } from "@/shared/api/tauriPersonas";
import type { AgentPersona } from "@/shared/api/types";

const STARTER_PERSONA_NAMES = ["Fizz", "Honey", "Bumble"];

export function StarterTeamCards() {
  const [personas, setPersonas] = React.useState<AgentPersona[]>([]);

  React.useEffect(() => {
    void listPersonas()
      .then((availablePersonas) =>
        setPersonas(
          STARTER_PERSONA_NAMES.flatMap((name) => {
            const persona = availablePersonas.find(
              (candidate) => candidate.displayName === name,
            );
            return persona ? [persona] : [];
          }),
        ),
      )
      .catch(() => setPersonas([]));
  }, []);

  if (personas.length === 0) return null;

  return (
    <div className="flex justify-center gap-5" data-testid="starter-team-cards">
      {personas.map((persona) => (
        <div className="flex w-20 flex-col items-center gap-2" key={persona.id}>
          <ProfileAvatar
            avatarUrl={persona.avatarUrl}
            className="h-14 w-14"
            label={persona.displayName}
          />
          <span className="text-sm font-medium">{persona.displayName}</span>
        </div>
      ))}
    </div>
  );
}
