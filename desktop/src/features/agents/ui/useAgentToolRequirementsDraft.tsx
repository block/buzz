import * as React from "react";

import type {
  AgentToolRequirement,
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";
import { AgentToolsSection } from "./AgentToolsSection";
import { agentToolRequirementIssues } from "./agentToolRequirements";

export function useAgentToolRequirementsDraft({
  disabled,
  initialValues,
  onUserChange,
  open,
}: {
  disabled: boolean;
  initialValues: CreatePersonaInput | UpdatePersonaInput | null;
  onUserChange: () => void;
  open: boolean;
}) {
  const [requirements, setRequirements] = React.useState<
    AgentToolRequirement[]
  >(initialValues?.toolRequirements ?? []);

  React.useEffect(() => {
    if (open && initialValues) {
      setRequirements(initialValues.toolRequirements ?? []);
    }
  }, [initialValues, open]);

  const issues = agentToolRequirementIssues(requirements);
  return {
    requirements,
    valid: issues.length === 0,
    section: (
      <AgentToolsSection
        disabled={disabled}
        issues={issues}
        onChange={(nextRequirements) => {
          onUserChange();
          setRequirements(nextRequirements);
        }}
        value={requirements}
      />
    ),
  };
}
