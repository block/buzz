import { useQueryClient } from "@tanstack/react-query";

import { createChannelManagedAgents } from "@/features/agents/channelAgents";
import {
  useAcpRuntimesQuery,
  usePersonasQuery,
  useTeamsQuery,
} from "@/features/agents/hooks";
import {
  buildTemplateAgentInputs,
  dataAfterInitialLoad,
} from "@/features/channel-templates/templateAgentInputs";
import { useLastRuntime } from "@/features/agents/lib/useLastRuntime";
import { useChannelTemplatesQuery } from "@/features/channel-templates/hooks";
import { setCanvas } from "@/shared/api/tauri";
import type {
  AcpRuntime,
  AcpRuntimeCatalogEntry,
  ChannelTemplate,
} from "@/shared/api/types";

function isAvailableRuntime(
  runtime: AcpRuntimeCatalogEntry,
): runtime is AcpRuntime {
  return runtime.availability === "available";
}

export function buildProjectFolderCanvas(
  projectFolders: string | readonly string[],
): string {
  const folders =
    typeof projectFolders === "string" ? [projectFolders] : projectFolders;
  const heading = folders.length === 1 ? "Project folder" : "Project folders";
  const rows = folders.map((folder) => `- \`${folder}\``).join("\n");
  return `## ${heading}\n\n${rows}`;
}

export function buildWorktreeCanvas(
  projectFolders: string | readonly string[],
  worktree: NonNullable<ChannelTemplate["worktree"]>,
): string {
  const folders =
    typeof projectFolders === "string" ? [projectFolders] : projectFolders;
  return [
    "## Workspace instructions",
    "",
    ...folders.map(
      (folder) =>
        `- Use \`${folder}\` for review only. Do not make changes directly in it.`,
    ),
    `- Before starting new work in any repository, fetch its latest \`${worktree.baseBranch}\` branch from origin.`,
    `- For each repository that needs changes, create a new worktree under \`${worktree.location}\` based on the latest \`origin/${worktree.baseBranch}\`.`,
    "- Make all changes in that new worktree.",
  ].join("\n");
}

export function useApplyTemplate() {
  const queryClient = useQueryClient();
  const channelTemplatesQuery = useChannelTemplatesQuery();
  const acpRuntimesQuery = useAcpRuntimesQuery();
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
    if (!template) return;
    const projectFolders =
      template.projectFolders.length > 0
        ? template.projectFolders
        : template.projectFolder
          ? [template.projectFolder]
          : [];
    const sections: string[] = [];
    if (projectFolders.length > 0 && template.worktree) {
      sections.push(buildWorktreeCanvas(projectFolders, template.worktree));
    } else if (projectFolders.length > 0) {
      sections.push(buildProjectFolderCanvas(projectFolders));
    }
    if (template.canvasTemplate) {
      sections.push(
        template.canvasTemplate
          .replace(/\{channel\.name\}/g, channelName)
          .replace(/\{template\.name\}/g, template.name),
      );
    }
    if (sections.length === 0) return;
    const content = sections.join("\n\n");
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

    try {
      const [allPersonas, allTeams, runtimeCatalog] = await Promise.all([
        dataAfterInitialLoad(personasQuery),
        dataAfterInitialLoad(teamsQuery),
        dataAfterInitialLoad(acpRuntimesQuery),
      ]);
      const runtimes = runtimeCatalog.filter(isAvailableRuntime);
      if (runtimes.length === 0) return; // No runtimes — skip silently

      const inputs = buildTemplateAgentInputs({
        template,
        personas: allPersonas,
        teams: allTeams,
        runtimes,
        lastRuntimeId,
      });
      if (inputs.length === 0) return;

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
