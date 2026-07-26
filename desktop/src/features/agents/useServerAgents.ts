import React from "react";

import {
  deleteSpawnerAgentSpec,
  publishSpawnerAgentSpec,
  type SpawnerAgentStatus,
} from "@/shared/api/spawnerRelay";
import type { AgentPersona } from "@/shared/api/types";
import { buildRelocationPlan, type RelocatableAgent } from "./agentRelocation";
import { personaDTag, slugFromName, useSpawners } from "./spawnerPreference";
import { spawnerStatusKey, useSpawnerStatuses } from "./spawnerStatusStore";

/**
 * Persona reference for a spec.
 *
 * Never the prompt itself: a kind:30178 spec is world-readable, so inlining a
 * system prompt would publish it to the whole community. The prompt travels
 * over the encrypted kind:24201 handshake at approval time instead
 * (`setSpawnerPromptResolver`).
 *
 * The reference is the persona's *relay* address, which is not its local id —
 * `builtin:fizz` is published as `builtin-fizz`, because the relay rejects a
 * colon in a `d` tag. A built-in that was never published simply resolves to
 * nothing on the relay, which is fine: the handshake supplies the prompt.
 */
function promptFieldsFor(persona: AgentPersona) {
  return { personaId: personaDTag(persona.id) };
}

/** A server-hosted agent as the Agents screen renders it. */
export type ServerAgent = {
  slug: string;
  spawnerPubkey: string;
  status: SpawnerAgentStatus;
};

/**
 * Server-hosted agents and the actions that manage them.
 *
 * Reads the list from kind:30179 status rather than from the specs this device
 * published: a spec is desired state that may have been created on another
 * device, and status is what the spawner is actually doing. Anything the
 * configured spawner reports is shown, so a phone sees the agents a laptop
 * created.
 */
export function useServerAgents() {
  const spawners = useSpawners();
  const statuses = useSpawnerStatuses();
  const [isPending, setIsPending] = React.useState(false);

  const agents = React.useMemo<ServerAgent[]>(() => {
    if (spawners.length === 0) return [];
    return [...statuses.entries()]
      .flatMap(([key, status]) => {
        // Status from a spawner this device is not connected to belongs to
        // somebody else's deployment, or to an impostor publishing at their own
        // NIP-33 address. Either way it is not this user's agent.
        const spawnerPubkey = spawners.find((s) => key.startsWith(`${s}/`));
        if (!spawnerPubkey) return [];
        return [
          {
            slug: key.slice(spawnerPubkey.length + 1),
            spawnerPubkey,
            status,
          },
        ];
      })
      .sort(
        (a, b) =>
          a.spawnerPubkey.localeCompare(b.spawnerPubkey) ||
          a.slug.localeCompare(b.slug),
      );
  }, [spawners, statuses]);

  /**
   * Create a server agent from a persona.
   *
   * The spec carries `personaId` rather than the prompt itself, matching how
   * kind:30177 slims against kind:30175 — the spawner resolves the prompt from
   * the persona, so it lives in exactly one place and editing the persona
   * updates the server agent too. Built-in personas are the exception: they
   * ship with the app and have no kind:30175 event, so the spawner could never
   * read them — their prompt goes inline.
   */
  const create = React.useCallback(
    async (
      persona: AgentPersona,
      spawnerPubkey: string,
      options?: { slug?: string },
    ) => {
      if (!spawnerPubkey) throw new Error("No spawner selected.");
      const slug = options?.slug ?? slugFromName(persona.displayName);
      if (!slug) {
        throw new Error(
          "Could not derive a name for this agent. Rename it using letters or digits.",
        );
      }
      setIsPending(true);
      try {
        await publishSpawnerAgentSpec({
          slug,
          spawnerPubkey,
          spec: {
            name: persona.displayName,
            ...promptFieldsFor(persona),
            parallelism: 1,
            respondTo: "anyone",
            enabled: true,
          },
        });
        return slug;
      } finally {
        setIsPending(false);
      }
    },
    [],
  );

  /**
   * Move an agent that already exists on this device onto a spawner.
   *
   * Distinct from `create`, which mints a new key: this publishes the existing
   * agent's *public* key on the spec, which is what tells the spawner to ask
   * for that identity's secret over the encrypted handshake rather than
   * generate a stranger. The local process is stopped later, by the attestation
   * store, once the handshake response has actually gone out.
   */
  const relocate = React.useCallback(
    async (agent: RelocatableAgent, spawnerPubkey: string) => {
      if (!spawnerPubkey) throw new Error("No spawner selected.");
      const { slug, spec } = buildRelocationPlan(agent);
      setIsPending(true);
      try {
        await publishSpawnerAgentSpec({ slug, spawnerPubkey, spec });
        return slug;
      } finally {
        setIsPending(false);
      }
    },
    [],
  );

  /**
   * Stop or resume an agent without touching its identity.
   *
   * Republishes the spec with `enabled` flipped rather than deleting it, so the
   * agent keeps its pubkey, its attestation, and its workspace volume — a
   * resumed agent is the same agent, not a stranger wearing its name.
   */
  const setEnabled = React.useCallback(
    async (agent: ServerAgent, enabled: boolean, persona?: AgentPersona) => {
      setIsPending(true);
      try {
        await publishSpawnerAgentSpec({
          slug: agent.slug,
          spawnerPubkey: agent.spawnerPubkey,
          spec: {
            name: persona?.displayName ?? agent.slug,
            // A definition-less spec needs an inline prompt to validate; this
            // placeholder only applies when the persona is gone, which is
            // itself a broken state the spawner will report.
            ...(persona
              ? promptFieldsFor(persona)
              : { systemPrompt: "Server-hosted Buzz agent." }),
            parallelism: 1,
            respondTo: "anyone",
            enabled,
          },
        });
      } finally {
        setIsPending(false);
      }
    },
    [],
  );

  /** Permanently remove an agent: its container, volume, and key. */
  const remove = React.useCallback(async (agent: ServerAgent) => {
    setIsPending(true);
    try {
      await deleteSpawnerAgentSpec({
        slug: agent.slug,
        spawnerPubkey: agent.spawnerPubkey,
      });
    } finally {
      setIsPending(false);
    }
  }, []);

  /**
   * Whether a persona is already deployed to a given spawner.
   *
   * Scoped per spawner, not globally: deploying the same persona to two
   * different hosts is a legitimate thing to want, and only a collision on the
   * *same* spawner would overwrite a running agent's spec.
   */
  const hasServerAgent = React.useCallback(
    (persona: AgentPersona, spawnerPubkey: string) => {
      const slug = slugFromName(persona.displayName);
      if (!slug) return false;
      return statuses.has(spawnerStatusKey(spawnerPubkey, slug));
    },
    [statuses],
  );

  return {
    spawners,
    agents,
    isPending,
    create,
    relocate,
    setEnabled,
    remove,
    hasServerAgent,
  };
}
