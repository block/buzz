import React from "react";

import { usePersonasQuery } from "@/features/agents/hooks";
import type { SpawnerPromptMaterial } from "@/shared/api/tauriSpawner";
import {
  ensureSpawnerAttestationSubscription,
  setSpawnerPromptResolver,
} from "./spawnerAttestationStore";
import { slugFromName } from "./spawnerPreference";
import { ensureSpawnerDirectorySubscription } from "./spawnerDirectoryStore";
import { ensureSpawnerStatusSubscription } from "./spawnerStatusStore";

/**
 * Opens the owner-scoped spawner subscriptions: kind:24201 attestation
 * handshakes, kind:30179 status, and kind:10180 spawner announcements.
 *
 * Mounted app-wide, once, next to `useAgentObserverIngestion`. Both
 * subscriptions are owner-global rather than screen-scoped, and the attestation
 * one in particular must be live everywhere: kind 24201 is ephemeral, so a
 * request that arrives while the Agents screen is unmounted is gone for good and
 * the agent sits at `pending_attestation` until the spawner re-sends.
 *
 * Deliberately unguarded by `startupReady` for the same reason — the stores
 * resolve identity themselves and no-op until one exists.
 */
export function useSpawnerIngestion(): void {
  React.useEffect(() => {
    void ensureSpawnerAttestationSubscription();
    void ensureSpawnerStatusSubscription();
    void ensureSpawnerDirectorySubscription();
  }, []);

  // Prompt material is sent over the encrypted handshake rather than published
  // on the world-readable spec, so the answer has to be assembled here, where
  // the persona library is in scope. Registered as a resolver so the store
  // stays independent of the persona layer.
  const personas = usePersonasQuery().data;
  React.useEffect(() => {
    if (!personas) {
      setSpawnerPromptResolver(null);
      return;
    }
    setSpawnerPromptResolver((specSlug): SpawnerPromptMaterial | null => {
      // Specs are named from the persona's display name, so that mapping is
      // what identifies which persona an inbound request is about.
      const persona = personas.find(
        (candidate) => slugFromName(candidate.displayName) === specSlug,
      );
      if (!persona) return null;
      return {
        system_prompt: persona.systemPrompt || undefined,
        model: persona.model ?? undefined,
        provider: persona.provider ?? undefined,
      };
    });
    return () => setSpawnerPromptResolver(null);
  }, [personas]);
}
