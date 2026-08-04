import { Plus, Trash2, Wrench } from "lucide-react";

import type { AgentToolRequirement } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import { Input } from "@/shared/ui/input";
import type { AgentToolRequirementIssue } from "./agentToolRequirements";

function newRequirementId() {
  return `tool_${crypto.randomUUID().replaceAll("-", "")}`;
}

export function AgentToolsSection({
  disabled,
  issues = [],
  onChange,
  value,
}: {
  disabled: boolean;
  issues?: AgentToolRequirementIssue[];
  onChange: (value: AgentToolRequirement[]) => void;
  value: AgentToolRequirement[];
}) {
  function update(
    id: string,
    patch: Partial<Omit<AgentToolRequirement, "id">>,
  ) {
    onChange(
      value.map((requirement) =>
        requirement.id === id ? { ...requirement, ...patch } : requirement,
      ),
    );
  }

  function issueFor(index: number, field: AgentToolRequirementIssue["field"]) {
    return issues.find(
      (issue) => issue.index === index && issue.field === field,
    );
  }

  return (
    <section className="space-y-3" data-testid="agent-tools-section">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <Wrench className="h-4 w-4 text-muted-foreground" />
            <h3 className="text-base font-semibold text-foreground">Tools</h3>
          </div>
          <p className="mt-1 text-xs text-muted-foreground">
            List what this template needs. Each agent connects those tools from
            its Project.
          </p>
        </div>
        <Button
          disabled={disabled || value.length >= 32}
          onClick={() =>
            onChange([
              ...value,
              {
                id: newRequirementId(),
                label: "",
                capability: "",
                required: true,
              },
            ])
          }
          size="sm"
          type="button"
          variant="outline"
        >
          <Plus className="h-4 w-4" />
          Add tool
        </Button>
      </div>

      {value.length === 0 ? (
        <div className="rounded-xl border border-dashed border-border/70 px-4 py-3 text-xs text-muted-foreground">
          This template does not need any connected tools.
        </div>
      ) : (
        <div className="divide-y divide-border/60 rounded-xl border border-border/70">
          {value.map((requirement, index) => {
            const rowIssue = issueFor(index, "row");
            const labelIssue = issueFor(index, "label");
            const capabilityIssue = issueFor(index, "capability");
            return (
              <div className="space-y-3 p-3" key={requirement.id}>
                {rowIssue ? (
                  <p className="text-xs text-destructive" role="alert">
                    {rowIssue.message}
                  </p>
                ) : null}
                <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto]">
                  <div className="space-y-1.5">
                    <label
                      className="text-xs font-medium text-foreground"
                      htmlFor={`agent-tool-label-${requirement.id}`}
                    >
                      Tool
                    </label>
                    <Input
                      aria-describedby={
                        labelIssue
                          ? `agent-tool-label-error-${requirement.id}`
                          : undefined
                      }
                      aria-invalid={Boolean(labelIssue)}
                      disabled={disabled}
                      id={`agent-tool-label-${requirement.id}`}
                      onChange={(event) =>
                        update(requirement.id, { label: event.target.value })
                      }
                      placeholder="Name this tool"
                      value={requirement.label}
                    />
                    {labelIssue ? (
                      <p
                        className="text-xs text-destructive"
                        id={`agent-tool-label-error-${requirement.id}`}
                        role="alert"
                      >
                        {labelIssue.message}
                      </p>
                    ) : null}
                  </div>

                  <div className="space-y-1.5">
                    <label
                      className="text-xs font-medium text-foreground"
                      htmlFor={`agent-tool-capability-${requirement.id}`}
                    >
                      Capability ID
                    </label>
                    <Input
                      aria-describedby={
                        capabilityIssue
                          ? `agent-tool-capability-error-${requirement.id}`
                          : `agent-tool-capability-help-${requirement.id}`
                      }
                      aria-invalid={Boolean(capabilityIssue)}
                      autoCapitalize="off"
                      autoCorrect="off"
                      className="font-mono text-xs"
                      disabled={disabled}
                      id={`agent-tool-capability-${requirement.id}`}
                      onChange={(event) =>
                        update(requirement.id, {
                          capability: event.target.value.trim(),
                        })
                      }
                      placeholder="mcp.tool."
                      spellCheck={false}
                      value={requirement.capability}
                    />
                    {capabilityIssue ? (
                      <p
                        className="text-xs text-destructive"
                        id={`agent-tool-capability-error-${requirement.id}`}
                        role="alert"
                      >
                        {capabilityIssue.message}
                      </p>
                    ) : (
                      <p
                        className="text-xs text-muted-foreground"
                        id={`agent-tool-capability-help-${requirement.id}`}
                      >
                        Copy this from a tested Project connection.
                      </p>
                    )}
                  </div>

                  <Button
                    aria-label={`Remove ${requirement.label || `tool ${index + 1}`}`}
                    className="mt-6 size-9"
                    disabled={disabled}
                    onClick={() =>
                      onChange(
                        value.filter((item) => item.id !== requirement.id),
                      )
                    }
                    size="icon"
                    type="button"
                    variant="ghost"
                  >
                    <Trash2 className="h-4 w-4" />
                  </Button>
                </div>

                <label
                  className="flex min-h-9 items-center gap-2 text-xs text-foreground"
                  htmlFor={`agent-tool-required-${requirement.id}`}
                >
                  <Checkbox
                    checked={requirement.required}
                    disabled={disabled}
                    id={`agent-tool-required-${requirement.id}`}
                    onCheckedChange={(checked) =>
                      update(requirement.id, { required: checked === true })
                    }
                  />
                  Required before launch
                </label>
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
}
