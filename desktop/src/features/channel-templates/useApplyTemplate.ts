import { useQueryClient } from "@tanstack/react-query";

import {
  createChannelManagedAgents,
  type CreateChannelManagedAgentInput,
} from "@/features/agents/channelAgents";
import {
  useAvailableAcpRuntimes,
  usePersonasQuery,
  useTeamsQuery,
} from "@/features/agents/hooks";
import { resolvePersonaRuntime } from "@/features/agents/lib/resolvePersonaRuntime";
import { resolveTeamPersonas } from "@/features/agents/lib/teamPersonas";
import { useLastRuntime } from "@/features/agents/lib/useLastRuntime";
import { useChannelTemplatesQuery } from "@/features/channel-templates/hooks";
import { setCanvas } from "@/shared/api/tauri";
import type { ChannelTemplate } from "@/shared/api/types";

/**
 * TemplateBackend omits `config` — supply an empty object for provider backends.
 */
function toManagedBackend(
  backend: ChannelTemplate["agents"]["personas"][number]["backend"],
): CreateChannelManagedAgentInput["backend"] {
  if (!backend || backend.type === "local") return { type: "local" };
  return { type: "provider", id: backend.id, config: {} };
}

export function useApplyTemplate() {
  const queryClient = useQueryClient();
  const channelTemplatesQuery = useChannelTemplatesQuery();
  const acpRuntimesQuery = useAvailableAcpRuntimes();
  const personasQuery = usePersonasQuery();
  const teamsQuery = useTeamsQuery();
  const { lastRuntimeId } = useLastRuntime();

  async function applyCanvas(
    templateId: string | undefined,
    channelId: string,
    channelName: string,
  ) {
    if (!templateId) return;
    const template = channelTemplatesQuery.data?.find(
      (t) => t.id === templateId,
    );
    if (!template?.canvasTemplate) return;
    const content = template.canvasTemplate
      .replace(/\{channel\.name\}/g, channelName)
      .replace(/\{template\.name\}/g, template.name);
    try {
      await setCanvas({ channelId, content });
    } catch {
      // Canvas is best-effort — don't block navigation
    }
  }

  async function applyAgents(
    templateId: string | undefined,
    channelId: string,
  ) {
    if (!templateId) return;
    const template = channelTemplatesQuery.data?.find(
      (t) => t.id === templateId,
    );
    if (!template) return;
    const { personas: templatePersonas, teams: templateTeams } =
      template.agents;
    if (templatePersonas.length === 0 && templateTeams.length === 0) return;

    const allPersonas = personasQuery.data ?? [];
    const allTeams = teamsQuery.data ?? [];
    const runtimes = acpRuntimesQuery.data ?? [];
    if (runtimes.length === 0) return; // No runtimes — skip silently

    // Resolve default provider: user's last-used preference, or first available
    const defaultProvider =
      runtimes.find((p) => p.id === lastRuntimeId) ?? runtimes[0] ?? null;
    if (!defaultProvider) return;

    const seenPersonaIds = new Set<string>();
    const inputs: CreateChannelManagedAgentInput[] = [];

    // Direct personas from template
    for (const entry of templatePersonas) {
      const persona = allPersonas.find((p) => p.id === entry.personaId);
      if (!persona) continue;
      if (seenPersonaIds.has(persona.id)) continue;
      seenPersonaIds.add(persona.id);
      const resolved = resolvePersonaRuntime(
        entry.runtime ?? persona.runtime,
        runtimes,
        defaultProvider,
      );
      inputs.push({
        runtime: resolved.runtime ?? defaultProvider,
        name: persona.displayName,
        personaId: persona.id,
        systemPrompt: persona.systemPrompt,
        avatarUrl: persona.avatarUrl ?? undefined,
        model: entry.model ?? persona.model ?? undefined,
        role: "bot",
        backend: toManagedBackend(entry.backend),
      });
    }

    // Team-expanded personas (skip dupes)
    for (const teamEntry of templateTeams) {
      const team = allTeams.find((t) => t.id === teamEntry.teamId);
      if (!team) continue;
      const { resolvedPersonas } = resolveTeamPersonas(team, allPersonas);
      for (const persona of resolvedPersonas) {
        if (seenPersonaIds.has(persona.id)) continue;
        seenPersonaIds.add(persona.id);
        const resolved = resolvePersonaRuntime(
          teamEntry.runtime ?? persona.runtime,
          runtimes,
          defaultProvider,
        );
        inputs.push({
          runtime: resolved.runtime ?? defaultProvider,
          name: persona.displayName,
          personaId: persona.id,
          systemPrompt: persona.systemPrompt,
          avatarUrl: persona.avatarUrl ?? undefined,
          model: teamEntry.model ?? persona.model ?? undefined,
          role: "bot",
          backend: toManagedBackend(teamEntry.backend),
        });
      }
    }

    if (inputs.length === 0) return;

    try {
      const result = await createChannelManagedAgents(channelId, inputs);
      if (result.failures.length > 0) {
        const { toast } = await import("sonner");
        toast.warning(
          result.failures.length === 1
            ? "1 agent from the template could not be created"
            : `${result.failures.length} agents from the template could not be created`,
        );
      }
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: ["channels", channelId, "members"],
        }),
        queryClient.invalidateQueries({ queryKey: ["managed-agents"] }),
        queryClient.invalidateQueries({ queryKey: ["relay-agents"] }),
      ]);
    } catch {
      // Agent creation is best-effort — don't block navigation
    }
  }

  return { applyCanvas, applyAgents };
}
