import * as React from "react";

import type { AgentPersona } from "@/shared/api/types";

const STORAGE_KEY = "buzz:bot-recents";
const MAX_RECENTS = 8;

// Default persona display names to seed the list when empty.
// These are resolved to IDs by the consumer.
export const DEFAULT_PERSONA_NAMES = ["Fizz", "Honey", "Bumble"] as const;

export function pickQuickBotPersonas(
  personas: readonly AgentPersona[],
  recentIds: readonly string[],
  maxCount = 3,
) {
  if (personas.length === 0) {
    return [];
  }

  const resolved: AgentPersona[] = [];

  const addPersona = (persona: AgentPersona | undefined) => {
    if (!persona || resolved.some((candidate) => candidate.id === persona.id)) {
      return;
    }

    resolved.push(persona);
  };

  for (const id of recentIds) {
    if (resolved.length >= maxCount) {
      break;
    }

    addPersona(personas.find((persona) => persona.id === id));
  }

  for (const name of DEFAULT_PERSONA_NAMES) {
    if (resolved.length >= maxCount) {
      break;
    }

    addPersona(
      personas.find(
        (persona) => persona.displayName.toLowerCase() === name.toLowerCase(),
      ),
    );
  }

  for (const persona of personas) {
    if (resolved.length >= maxCount) {
      break;
    }

    addPersona(persona);
  }

  return resolved;
}

export function useBotRecents(): {
  recentIds: string[];
  pushRecent: (personaId: string) => void;
} {
  const [recentIds, setRecentIds] = React.useState<string[]>(() => {
    try {
      const raw = localStorage.getItem(STORAGE_KEY);
      return raw ? (JSON.parse(raw) as string[]) : [];
    } catch {
      return [];
    }
  });

  const pushRecent = React.useCallback((personaId: string) => {
    setRecentIds((prev) => {
      const next = [personaId, ...prev.filter((id) => id !== personaId)].slice(
        0,
        MAX_RECENTS,
      );
      try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
      } catch {
        // localStorage full — ignore
      }
      return next;
    });
  }, []);

  return { recentIds, pushRecent };
}
