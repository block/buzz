import * as React from "react";
import {
  Bot,
  Boxes,
  Check,
  Circle,
  Gem,
  Loader2,
  Network,
  Sparkles,
} from "lucide-react";

import type { AgentModelInfo, AgentPersona } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";

const DEFAULT_MODEL_KEY = "__sprout_runtime_default__";

export type AgentMentionModelTarget = {
  key: string;
  displayName: string;
  personaId: string | null;
  avatarUrl: string | null;
  currentModel: string | null;
  defaultModel: string | null;
  selectedModel: string | null;
  modelOptions: AgentModelInfo[];
  loadError: string | null;
  isNewMention: boolean;
  showModelInTrigger: boolean;
  willCreateNewInstance: boolean;
};

type AgentMentionModelSelectorProps = {
  disabled: boolean;
  error: string | null;
  isLoading: boolean;
  isLoadingPersonas: boolean;
  onModelChange: (key: string, model: string | null) => void;
  onPersonaSelect: (persona: AgentPersona) => void;
  onTriggerMouseDown: () => void;
  personas: AgentPersona[];
  targets: AgentMentionModelTarget[];
};

type ProviderHint = {
  className: string;
  icon: React.ReactNode;
  label: string;
};

function modelName(model: AgentModelInfo) {
  return model.name?.trim() || model.id;
}

function getSelectedModel(target: AgentMentionModelTarget | null) {
  if (!target) {
    return null;
  }

  if (target.selectedModel === null) {
    return null;
  }

  return (
    target.modelOptions.find((model) => model.id === target.selectedModel) ?? {
      id: target.selectedModel,
      name: null,
      description: null,
    }
  );
}

function triggerLabel(target: AgentMentionModelTarget | null) {
  if (!target?.showModelInTrigger) {
    return null;
  }

  const selectedModel = getSelectedModel(target);
  if (selectedModel) {
    return modelName(selectedModel);
  }

  return target.defaultModel ? `${target.defaultModel} default` : "Auto";
}

function defaultModelDescription(defaultModel: string | null) {
  return defaultModel ? `Uses ${defaultModel}` : "Use the runtime default";
}

function getProviderHint(model: AgentModelInfo | null): ProviderHint {
  const text = model ? `${model.id} ${model.name ?? ""}`.toLowerCase() : "";

  if (text.includes("claude") || text.includes("anthropic")) {
    return {
      className: "text-[#d97757]",
      icon: <Sparkles aria-hidden className="h-3.5 w-3.5" />,
      label: "Anthropic",
    };
  }

  if (text.includes("gpt") || text.includes("openai")) {
    return {
      className: "text-foreground",
      icon: <Bot aria-hidden className="h-3.5 w-3.5" />,
      label: "OpenAI",
    };
  }

  if (text.includes("gemini") || text.includes("google")) {
    return {
      className: "text-[#4285f4]",
      icon: <Gem aria-hidden className="h-3.5 w-3.5" />,
      label: "Google",
    };
  }

  if (text.includes("databricks")) {
    return {
      className: "text-[#ee3d2c]",
      icon: <Boxes aria-hidden className="h-3.5 w-3.5" />,
      label: "Databricks",
    };
  }

  if (
    text.includes("hf://") ||
    text.includes("gguf") ||
    text.includes("llama") ||
    text.includes("mistral") ||
    text.includes("mesh")
  ) {
    return {
      className: "text-emerald-600",
      icon: <Network aria-hidden className="h-3.5 w-3.5" />,
      label: "Mesh",
    };
  }

  return {
    className: "text-muted-foreground",
    icon: <Sparkles aria-hidden className="h-3.5 w-3.5" />,
    label: "Model",
  };
}

function modelDescription(model: AgentModelInfo) {
  const provider = getProviderHint(model).label;
  const label = modelName(model);
  if (model.description?.trim()) {
    return model.description;
  }

  return model.id !== label ? `${provider} · ${model.id}` : provider;
}

export function AgentMentionModelSelector({
  disabled,
  error,
  isLoading,
  isLoadingPersonas,
  onModelChange,
  onPersonaSelect,
  onTriggerMouseDown,
  personas,
  targets,
}: AgentMentionModelSelectorProps) {
  const [open, setOpen] = React.useState(false);
  const [activeTargetKey, setActiveTargetKey] = React.useState<string | null>(
    null,
  );
  const [pendingPersonaId, setPendingPersonaId] = React.useState<string | null>(
    null,
  );
  const activeTarget =
    targets.find((target) => target.key === activeTargetKey) ??
    targets[0] ??
    null;
  const label = triggerLabel(activeTarget);
  const selectedModel = getSelectedModel(activeTarget);
  const provider = getProviderHint(selectedModel);

  React.useEffect(() => {
    if (targets.length === 0) {
      setActiveTargetKey(null);
      return;
    }

    setActiveTargetKey((current) => {
      if (current && targets.some((target) => target.key === current)) {
        return current;
      }

      const pendingTarget = pendingPersonaId
        ? targets.find((target) => target.personaId === pendingPersonaId)
        : null;
      return pendingTarget?.key ?? targets[0].key;
    });
  }, [pendingPersonaId, targets]);

  React.useEffect(() => {
    if (
      pendingPersonaId &&
      targets.some((target) => target.personaId === pendingPersonaId)
    ) {
      setPendingPersonaId(null);
    }
  }, [pendingPersonaId, targets]);

  const handlePersonaSelect = React.useCallback(
    (persona: AgentPersona) => {
      const existingTarget = targets.find(
        (target) => target.personaId === persona.id,
      );
      if (existingTarget) {
        setActiveTargetKey(existingTarget.key);
        return;
      }

      setPendingPersonaId(persona.id);
      onPersonaSelect(persona);
      window.setTimeout(() => setOpen(true), 0);
    },
    [onPersonaSelect, targets],
  );

  const trigger = (
    <Button
      aria-label="Add an agent"
      className={cn(
        "h-8 border border-transparent bg-transparent text-muted-foreground shadow-none hover:bg-muted hover:text-foreground",
        "focus-visible:ring-1 focus-visible:ring-ring",
        label
          ? "max-w-[13rem] justify-start gap-1.5 rounded-full border-border/50 bg-muted/45 px-2.5 text-foreground"
          : "w-8 justify-center rounded-md px-0",
      )}
      data-testid="agent-model-selector-trigger"
      disabled={disabled}
      onMouseDown={onTriggerMouseDown}
      size="sm"
      type="button"
      variant="ghost"
    >
      {isLoading ? (
        <Loader2 aria-hidden className="h-3.5 w-3.5 animate-spin" />
      ) : label ? (
        <span className={provider.className}>{provider.icon}</span>
      ) : (
        <Bot aria-hidden className="h-3.5 w-3.5" />
      )}
      {label ? <span className="truncate">{label}</span> : null}
    </Button>
  );

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>{trigger}</PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent>Add an agent</TooltipContent>
      </Tooltip>
      <PopoverContent
        align="start"
        className="h-[min(24rem,50vh)] w-[min(34rem,calc(100vw-2rem))] overflow-hidden p-1"
        data-testid="agent-model-selector-popover"
        onCloseAutoFocus={(event) => event.preventDefault()}
      >
        <div className="grid h-full min-h-0 grid-cols-[minmax(0,13rem)_minmax(0,1fr)] gap-1 overflow-hidden">
          <AgentColumn
            activeTarget={activeTarget}
            isLoading={isLoadingPersonas}
            onPersonaSelect={handlePersonaSelect}
            onTargetSelect={setActiveTargetKey}
            pendingPersonaId={pendingPersonaId}
            personas={personas}
            targets={targets}
          />
          <ModelColumn
            activeTarget={activeTarget}
            error={error}
            isLoading={isLoading}
            onClose={() => setOpen(false)}
            onModelChange={onModelChange}
          />
        </div>
      </PopoverContent>
    </Popover>
  );
}

function AgentColumn({
  activeTarget,
  isLoading,
  onPersonaSelect,
  onTargetSelect,
  pendingPersonaId,
  personas,
  targets,
}: {
  activeTarget: AgentMentionModelTarget | null;
  isLoading: boolean;
  onPersonaSelect: (persona: AgentPersona) => void;
  onTargetSelect: (key: string) => void;
  pendingPersonaId: string | null;
  personas: AgentPersona[];
  targets: AgentMentionModelTarget[];
}) {
  const sortedPersonas = React.useMemo(
    () =>
      [...personas].sort((left, right) =>
        left.displayName.localeCompare(right.displayName),
      ),
    [personas],
  );
  const personaTargetById = React.useMemo(
    () =>
      new Map(
        targets
          .filter((target) => target.personaId)
          .map((target) => [target.personaId as string, target]),
      ),
    [targets],
  );
  const extraTargets = React.useMemo(
    () => targets.filter((target) => !target.personaId),
    [targets],
  );

  return (
    <div
      className="flex min-h-0 min-w-0 flex-col border-r border-border/70 p-1"
      data-col="agent"
    >
      <div className="shrink-0 px-2 py-1.5 text-sm font-semibold">Agent</div>
      {isLoading && sortedPersonas.length === 0 ? (
        <div className="flex min-h-0 flex-1 items-center gap-2 px-2 py-2 text-sm text-muted-foreground">
          <Loader2 aria-hidden className="h-4 w-4 animate-spin" />
          <span>Loading agents</span>
        </div>
      ) : sortedPersonas.length > 0 || extraTargets.length > 0 ? (
        <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain pr-1">
          <div className="space-y-0.5 p-1">
            {extraTargets.map((target) => (
              <AgentOptionButton
                avatarUrl={target.avatarUrl}
                isSelected={activeTarget?.key === target.key}
                key={target.key}
                label={target.displayName}
                onClick={() => onTargetSelect(target.key)}
                testId={`agent-model-selector-agent-${target.key}`}
              />
            ))}
            {sortedPersonas.map((persona) => {
              const target = personaTargetById.get(persona.id);
              const isSelected = target
                ? activeTarget?.key === target.key
                : pendingPersonaId === persona.id;
              return (
                <AgentOptionButton
                  avatarUrl={persona.avatarUrl}
                  description={
                    persona.model?.trim() || persona.runtime || "Agent"
                  }
                  isSelected={isSelected}
                  key={persona.id}
                  label={persona.displayName}
                  onClick={() =>
                    target
                      ? onTargetSelect(target.key)
                      : onPersonaSelect(persona)
                  }
                  testId={`agent-model-selector-persona-${persona.id}`}
                />
              );
            })}
          </div>
        </div>
      ) : (
        <div className="px-3 py-8 text-center text-sm text-muted-foreground">
          No agents available.
        </div>
      )}
    </div>
  );
}

function AgentOptionButton({
  avatarUrl,
  description,
  isSelected,
  label,
  onClick,
  testId,
}: {
  avatarUrl: string | null;
  description?: string;
  isSelected: boolean;
  label: string;
  onClick: () => void;
  testId: string;
}) {
  return (
    <button
      className={cn(
        "flex min-h-10 w-full min-w-0 items-center gap-2 overflow-hidden rounded-sm px-2 py-1.5 text-left text-sm transition-colors",
        "hover:bg-accent focus-visible:bg-accent focus-visible:outline-none",
        isSelected && "bg-accent",
      )}
      data-testid={testId}
      onClick={onClick}
      onMouseDown={(event) => event.preventDefault()}
      type="button"
    >
      <UserAvatar avatarUrl={avatarUrl} displayName={label} size="xs" />
      <span className="min-w-0 flex-1 overflow-hidden">
        <span className="block truncate">{label}</span>
        {description ? (
          <span className="block truncate text-xs text-muted-foreground">
            {description}
          </span>
        ) : null}
      </span>
      {isSelected ? (
        <Check aria-hidden className="h-4 w-4 shrink-0 text-muted-foreground" />
      ) : null}
    </button>
  );
}

function ModelColumn({
  activeTarget,
  error,
  isLoading,
  onClose,
  onModelChange,
}: {
  activeTarget: AgentMentionModelTarget | null;
  error: string | null;
  isLoading: boolean;
  onClose: () => void;
  onModelChange: (key: string, model: string | null) => void;
}) {
  const handleModelChange = React.useCallback(
    (model: string | null) => {
      if (!activeTarget) {
        return;
      }

      onModelChange(activeTarget.key, model);
      onClose();
    },
    [activeTarget, onClose, onModelChange],
  );

  return (
    <div className="flex min-h-0 min-w-0 flex-col p-1" data-col="model">
      <div className="flex shrink-0 items-center justify-between gap-2 px-2 py-1.5">
        <div className="text-sm font-semibold">Model</div>
        {isLoading ? (
          <Loader2
            aria-hidden
            className="h-3.5 w-3.5 animate-spin text-muted-foreground"
          />
        ) : null}
      </div>

      {error ? (
        <div className="mx-1 mb-1 rounded-md bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      ) : null}

      {activeTarget?.loadError ? (
        <div className="mx-1 mb-1 rounded-md bg-warning/10 px-3 py-2 text-sm text-warning">
          {activeTarget.loadError}
        </div>
      ) : null}

      {activeTarget ? (
        <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain pr-1">
          <div className="space-y-0.5 p-1">
            <ModelOptionButton
              description={defaultModelDescription(activeTarget.defaultModel)}
              icon={
                <Circle
                  aria-hidden
                  className="h-3.5 w-3.5 text-muted-foreground"
                />
              }
              isSelected={activeTarget.selectedModel === null}
              label="Runtime default"
              onClick={() => handleModelChange(null)}
              testId={`agent-model-selector-model-${activeTarget.key}-${DEFAULT_MODEL_KEY}`}
            />
            {activeTarget.modelOptions.map((model) => {
              const provider = getProviderHint(model);
              return (
                <ModelOptionButton
                  description={modelDescription(model)}
                  icon={
                    <span className={provider.className}>{provider.icon}</span>
                  }
                  isSelected={activeTarget.selectedModel === model.id}
                  key={model.id}
                  label={modelName(model)}
                  onClick={() => handleModelChange(model.id)}
                  testId={`agent-model-selector-model-${activeTarget.key}-${model.id}`}
                />
              );
            })}
          </div>
        </div>
      ) : (
        <div className="flex min-h-0 flex-1 items-center justify-center px-3 py-8 text-center text-sm text-muted-foreground">
          Choose an agent to pick a model.
        </div>
      )}
    </div>
  );
}

function ModelOptionButton({
  description,
  icon,
  isSelected,
  label,
  onClick,
  testId,
}: {
  description: string;
  icon: React.ReactNode;
  isSelected: boolean;
  label: string;
  onClick: () => void;
  testId: string;
}) {
  return (
    <button
      className={cn(
        "flex min-h-10 w-full min-w-0 items-center gap-2 overflow-hidden rounded-sm px-2 py-1.5 text-left text-sm transition-colors",
        "hover:bg-accent focus-visible:bg-accent focus-visible:outline-none",
        isSelected && "bg-accent",
      )}
      data-testid={testId}
      onClick={onClick}
      type="button"
    >
      <span className="flex h-5 w-5 shrink-0 items-center justify-center">
        {icon}
      </span>
      <span className="min-w-0 flex-1 overflow-hidden">
        <span className="block truncate">{label}</span>
        <span className="block truncate text-xs text-muted-foreground">
          {description}
        </span>
      </span>
      {isSelected ? (
        <Check aria-hidden className="h-4 w-4 shrink-0 text-muted-foreground" />
      ) : null}
    </button>
  );
}
