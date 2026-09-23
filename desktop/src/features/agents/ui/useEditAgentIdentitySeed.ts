import * as React from "react";

import type { AgentPersona } from "@/shared/api/types";

/**
 * Seeds description + instructions once per dialog open from the linked
 * persona (when present). Personas may resolve after open; avoid wiping
 * in-progress edits on later refetches.
 */
export function useEditAgentIdentitySeed(options: {
  agentPersonaId: string | null;
  agentSystemPrompt: string | null;
  linkedPersona: AgentPersona | null;
  open: boolean;
  setDescriptionDraft: (value: string) => void;
  setSystemPrompt: (value: string) => void;
}) {
  const {
    agentPersonaId,
    agentSystemPrompt,
    linkedPersona,
    open,
    setDescriptionDraft,
    setSystemPrompt,
  } = options;
  const identitySeededForOpen = React.useRef(false);

  React.useEffect(() => {
    if (!open) {
      identitySeededForOpen.current = false;
      return;
    }
    if (identitySeededForOpen.current) return;
    if (agentPersonaId != null && linkedPersona == null) return;
    setDescriptionDraft(linkedPersona?.description ?? "");
    setSystemPrompt(linkedPersona?.systemPrompt ?? agentSystemPrompt ?? "");
    identitySeededForOpen.current = true;
  }, [
    open,
    agentPersonaId,
    agentSystemPrompt,
    linkedPersona,
    linkedPersona?.description,
    linkedPersona?.systemPrompt,
    setDescriptionDraft,
    setSystemPrompt,
  ]);

  return {
    /** Call when the main open-reset effect runs so the seed can re-run. */
    resetIdentitySeed: () => {
      identitySeededForOpen.current = false;
    },
  };
}
