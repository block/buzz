import type { FeatureDefinition } from "@/shared/features/types";

/** Definitions available only in the protected internal application build. */
export const protectedFeatureDefinitions: FeatureDefinition[] = [
  {
    id: "bestie",
    name: "Assistant",
    description: "Personal assistant always close at hand — reminders, jobs, and chat",
    defaultEnabled: true,
    platforms: ["desktop"],
  },
  {
    id: "openclaw-workspace-mcp",
    name: "OpenClaw workspace",
    description:
      "OpenClaw workspace MCP grant + per-agent remote FS / skill pack mode",
    defaultEnabled: true,
    platforms: ["desktop"],
  },
];
