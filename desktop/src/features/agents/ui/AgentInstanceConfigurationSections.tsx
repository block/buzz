import type * as React from "react";
import { ChevronDown } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import { cn } from "@/shared/lib/cn";
import { AdvancedRequiredBadge } from "./AdvancedRequiredBadge";
import { AgentMcpConnectionsSection } from "./AgentMcpConnectionsSection";
import { EditAgentAdvancedFields } from "./EditAgentAdvancedFields";

const ADVANCED_FIELDS_MOTION_TRANSITION = {
  duration: 0.18,
  ease: [0.23, 1, 0.32, 1],
} as const;

type AdvancedFieldsProps = React.ComponentProps<typeof EditAgentAdvancedFields>;
type McpConnectionsProps = React.ComponentProps<
  typeof AgentMcpConnectionsSection
>;

export function AgentInstanceConfigurationSections({
  advancedFieldsProps,
  agentPubkey,
  badgeEnvVars,
  mcpConnectionsProps,
  open,
  onShowAdvancedFieldsChange,
  requiredEnvKeys,
  showAdvancedFields,
  showMcpConnections,
}: {
  advancedFieldsProps: AdvancedFieldsProps;
  agentPubkey: string;
  badgeEnvVars: Record<string, string>;
  mcpConnectionsProps: McpConnectionsProps;
  open: boolean;
  onShowAdvancedFieldsChange: (show: boolean) => void;
  requiredEnvKeys: readonly string[];
  showAdvancedFields: boolean;
  showMcpConnections: boolean;
}) {
  const shouldReduceMotion = useReducedMotion();
  const transition = shouldReduceMotion
    ? { duration: 0 }
    : ADVANCED_FIELDS_MOTION_TRANSITION;

  return (
    <>
      {showMcpConnections ? (
        <AgentMcpConnectionsSection
          {...mcpConnectionsProps}
          key={`${agentPubkey}:${open}`}
        />
      ) : null}

      <div className="space-y-3">
        <button
          aria-expanded={showAdvancedFields}
          className="inline-flex h-9 items-center gap-1.5 text-sm font-medium text-foreground transition-colors hover:text-foreground/80 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
          onClick={() => onShowAdvancedFieldsChange(!showAdvancedFields)}
          type="button"
        >
          <span>Advanced</span>
          <AdvancedRequiredBadge
            envVars={badgeEnvVars}
            requiredEnvKeys={requiredEnvKeys}
            testId="edit-agent-advanced-required-badge"
          />
          <ChevronDown
            className={cn(
              "h-4 w-4 text-muted-foreground transition-transform duration-150 ease-out",
              showAdvancedFields && "rotate-180",
            )}
          />
        </button>
        <AnimatePresence initial={false}>
          {showAdvancedFields ? (
            <motion.div
              animate={{ height: "auto", opacity: 1, scale: 1 }}
              className="origin-top overflow-hidden"
              exit={{ height: 0, opacity: 0, scale: 0.98 }}
              initial={{ height: 0, opacity: 0, scale: 0.98 }}
              key="edit-agent-advanced-fields"
              transition={transition}
            >
              <EditAgentAdvancedFields {...advancedFieldsProps} />
            </motion.div>
          ) : null}
        </AnimatePresence>
      </div>
    </>
  );
}
