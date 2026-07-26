import * as React from "react";
import { toast } from "sonner";

import type { AgentPersona } from "@/shared/api/types";
import { useSpawnerDirectory } from "../spawnerDirectoryStore";
import { useServerAgents } from "../useServerAgents";
import { spawnerLabel } from "./ServerAgentsSection";

/**
 * Publishes the spec that makes a persona run on a spawner, with feedback.
 *
 * Shared by every create surface so "Runs on: <server>" means the same thing
 * wherever it is chosen. No local instance is created and no binary is named:
 * the host decides what executes there.
 */
export function useDeployPersonaToSpawner() {
  const { create } = useServerAgents();
  const directory = useSpawnerDirectory();

  return React.useCallback(
    async (persona: AgentPersona, spawnerPubkey: string): Promise<boolean> => {
      try {
        await create(persona, spawnerPubkey);
        toast.success(
          `Deploying ${persona.displayName} to ${spawnerLabel(spawnerPubkey, directory)}. Approve the key when prompted.`,
        );
        return true;
      } catch (error) {
        toast.error(
          error instanceof Error
            ? error.message
            : `${persona.displayName} was created, but it could not be deployed to the server.`,
        );
        return false;
      }
    },
    [create, directory],
  );
}
