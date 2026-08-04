import type { AgentToolRequirement } from "@/shared/api/types";

const UNSAFE_RECORD_KEYS = new Set(["__proto__", "constructor", "prototype"]);
const MAX_REQUIREMENTS = 32;

export type AgentToolRequirementIssue = {
  field: "row" | "label" | "capability";
  index: number;
  message: string;
};

export function agentToolRequirementIssues(
  requirements: readonly AgentToolRequirement[],
): AgentToolRequirementIssue[] {
  const issues: AgentToolRequirementIssue[] = [];
  const ids = new Set<string>();

  if (requirements.length > MAX_REQUIREMENTS) {
    issues.push({
      field: "row",
      index: MAX_REQUIREMENTS,
      message: `A template can request up to ${MAX_REQUIREMENTS} tools.`,
    });
  }

  for (const [index, requirement] of requirements.entries()) {
    if (
      !/^[a-z0-9_.-]{1,64}$/.test(requirement.id) ||
      UNSAFE_RECORD_KEYS.has(requirement.id) ||
      ids.has(requirement.id)
    ) {
      issues.push({
        field: "row",
        index,
        message: "Remove this tool and add it again.",
      });
    }
    ids.add(requirement.id);

    const label = requirement.label.trim();
    if (
      label.length === 0 ||
      new TextEncoder().encode(label).length > 128 ||
      [...label].some((character) => /\p{Cc}/u.test(character))
    ) {
      issues.push({
        field: "label",
        index,
        message: "Enter a tool name under 128 characters.",
      });
    }

    if (
      new TextEncoder().encode(requirement.capability).length > 128 ||
      !/^mcp\.tool\.[A-Za-z0-9_.-]+$/.test(requirement.capability)
    ) {
      issues.push({
        field: "capability",
        index,
        message: "Enter a capability ID beginning with mcp.tool.",
      });
    }
  }

  return issues;
}

export function agentToolRequirementsValid(
  requirements: readonly AgentToolRequirement[],
) {
  return agentToolRequirementIssues(requirements).length === 0;
}
