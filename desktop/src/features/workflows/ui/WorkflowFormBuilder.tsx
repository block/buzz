import {
  ArrowDown,
  Check,
  ChevronDown,
  Plus,
  Trash2,
  X,
  Zap,
} from "lucide-react";
import { FocusScope } from "@radix-ui/react-focus-scope";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import * as React from "react";
import { createPortal } from "react-dom";

import type { Channel } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Input } from "@/shared/ui/input";
import { Switch } from "@/shared/ui/switch";
import { Textarea } from "@/shared/ui/textarea";
import { WorkflowEmojiField } from "./WorkflowEmojiField";
import { WorkflowMessageTextCondition } from "./WorkflowMessageTextConditionEditor";
import { WorkflowScheduleFields } from "./WorkflowScheduleFields";
import { WorkflowStepCard } from "./WorkflowStepCard";
import type { WorkflowEditorPane } from "./workflowEditorPane";
import { FieldLabel } from "./workflowFormPrimitives";
import {
  DEFAULT_FORM_STATE,
  ACTION_LABELS,
  SELECTABLE_ACTION_TYPES,
  SELECTABLE_TRIGGER_TYPES,
  TRIGGER_LABELS,
  formStateToYaml,
  nextStepId,
  supportsMessageTextCondition,
  withTriggerType,
  yamlToFormState,
} from "./workflowFormTypes";
import { defaultScheduleTrigger } from "./workflowSchedule";
import { readWorkflowDocumentFields } from "./workflowYamlDocument";
import type {
  ActionType,
  StepFormState,
  TriggerConfig,
  WorkflowFormState,
} from "./workflowFormTypes";

function TriggerConfigFields({
  disabled,
  trigger,
  onUpdate,
}: {
  disabled?: boolean;
  trigger: TriggerConfig;
  onUpdate: (trigger: TriggerConfig) => void;
}) {
  switch (trigger.on) {
    case "message_posted":
      return (
        <WorkflowMessageTextCondition
          disabled={disabled}
          onChange={(filter) => onUpdate({ ...trigger, filter })}
          value={trigger.filter ?? ""}
        />
      );
    case "diff_posted":
      return (
        <div className="space-y-1.5">
          <FieldLabel htmlFor="wf-trigger-filter">
            Condition (optional)
          </FieldLabel>
          <Input
            autoCapitalize="off"
            disabled={disabled}
            id="wf-trigger-filter"
            onChange={(event) =>
              onUpdate({ ...trigger, filter: event.target.value })
            }
            placeholder='e.g. str_contains(trigger_text, "deploy")'
            value={trigger.filter ?? ""}
          />
          <p className="text-xs text-muted-foreground">
            Evalexpr. Empty matches all events.
          </p>
        </div>
      );
    case "reaction_added":
      return (
        <div className="space-y-1.5">
          <FieldLabel htmlFor="wf-trigger-emoji">
            Emoji filter (optional)
          </FieldLabel>
          <WorkflowEmojiField
            ariaLabel="Choose trigger emoji"
            clearAriaLabel="Clear trigger emoji"
            disabled={disabled}
            id="wf-trigger-emoji"
            onChange={(emoji) => onUpdate({ ...trigger, emoji })}
            value={trigger.emoji ?? ""}
          />
          <p className="text-xs text-muted-foreground">
            Empty matches any reaction.
          </p>
        </div>
      );
    case "webhook":
      return (
        <p className="text-xs text-muted-foreground">
          A unique URL is generated after creation.
        </p>
      );
    case "schedule":
      return (
        <WorkflowScheduleFields
          disabled={disabled}
          onUpdate={onUpdate}
          trigger={trigger}
        />
      );
    default:
      return null;
  }
}

type WorkflowFormBuilderProps = {
  channels: Channel[];
  disabled?: boolean;
  nameLeadingContainer?: HTMLElement | null;
  mode: WorkflowEditorMode;
  onChange: (yaml: string) => void;
  onSelectedNodeChange: (pane: WorkflowEditorPane) => void;
  parseError: string | null;
  scopeField?: React.ReactNode;
  selectedNode: WorkflowEditorPane;
  workflowChannelId?: string | null;
  yaml: string;
};

export type WorkflowFormBuilderHandle = {
  addFirstStep: () => void;
  closeInspector: () => boolean;
  synchronizeYaml: (yaml: string) => void;
};

export type WorkflowEditorMode = "form" | "yaml";

function nodePosition(
  node: Exclude<WorkflowEditorPane, null>,
  steps: StepFormState[],
): number {
  if (node.type === "trigger") return 0;
  const index = steps.findIndex((step) => step.id === node.stepId);
  return index < 0 ? 0 : index + 1;
}

const inspectorContentVariants = {
  enter: (direction: number) => ({
    opacity: 0,
    y: direction < 0 ? 12 : -12,
  }),
  center: { opacity: 1, y: 0 },
  exit: (direction: number) => ({
    opacity: 0,
    y: direction < 0 ? -12 : 12,
  }),
};

function InspectorTypeMenu<T extends string>({
  ariaLabel,
  disabled,
  labels,
  onChange,
  options,
  value,
}: {
  ariaLabel: string;
  disabled?: boolean;
  labels: Record<T, string>;
  onChange: (value: T) => void;
  options: readonly T[];
  value: T;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          aria-label={ariaLabel}
          className="group inline-flex max-w-full items-center gap-1.5 rounded-md py-0.5 text-base font-semibold text-foreground outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
          data-value={value}
          disabled={disabled}
          type="button"
        >
          <span className="truncate">{labels[value]}</span>
          <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground opacity-60 transition-opacity group-hover:opacity-100" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" sideOffset={8}>
        {options.map((option) => (
          <DropdownMenuItem key={option} onSelect={() => onChange(option)}>
            <Check
              className={cn(
                "h-4 w-4",
                option === value ? "opacity-100" : "opacity-0",
              )}
            />
            {labels[option]}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function WorkflowNode({
  description,
  disabled,
  icon,
  label,
  number,
  onAddAfter,
  onClick,
  onRemove,
  selected,
  showTitle = true,
  subtitle,
  terminal,
  title,
}: {
  description: string;
  disabled?: boolean;
  icon?: React.ReactNode;
  label: string;
  number?: number;
  onAddAfter: (action: ActionType) => void;
  onClick: () => void;
  onRemove?: () => void;
  selected: boolean;
  showTitle?: boolean;
  subtitle?: string;
  terminal: boolean;
  title: string;
}) {
  const isNumbered = number !== undefined;
  const [addMenuOpen, setAddMenuOpen] = React.useState(false);

  return (
    <li className="flex flex-col items-center">
      <div className="group relative isolate w-full after:absolute after:left-full after:top-0 after:z-0 after:h-full after:w-12 after:content-['']">
        <button
          aria-label={label}
          aria-pressed={selected}
          className={cn(
            "relative z-20 flex w-full items-center gap-3 text-left transition-colors",
            "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
            "rounded-full bg-muted/25 p-3 outline outline-2 outline-offset-4 outline-muted-foreground/0",
            "data-[selected=true]:bg-muted/70 data-[selected=true]:outline-muted-foreground/20",
            "data-[selected=false]:hover:bg-muted/45",
          )}
          data-selected={selected}
          disabled={disabled}
          onClick={onClick}
          type="button"
        >
          <span
            className={cn(
              "flex h-9 w-9 shrink-0 items-center justify-center",
              "rounded-full bg-muted/60 text-muted-foreground",
              "data-[selected=true]:bg-foreground/15 data-[selected=true]:text-foreground",
              isNumbered && "text-sm font-semibold",
            )}
            data-selected={selected}
          >
            {isNumbered ? number : icon}
          </span>
          <span className="min-w-0 flex-1">
            {showTitle ? (
              <span className="block text-2xs font-semibold uppercase tracking-wide text-muted-foreground/70">
                {title}
              </span>
            ) : null}
            {subtitle ? (
              <span className="block truncate text-2xs font-semibold uppercase tracking-wide text-muted-foreground/70">
                {subtitle}
              </span>
            ) : null}
            <span className="block truncate text-sm font-semibold text-foreground">
              {description}
            </span>
          </span>
        </button>

        {onRemove ? (
          <Button
            aria-label={`Remove ${title}`}
            className="pointer-events-none absolute -right-8 top-1/2 z-10 h-8 w-8 -translate-y-1/2 rounded-full bg-transparent opacity-0 transition-all duration-200 group-focus-within:pointer-events-auto group-focus-within:translate-x-3 group-focus-within:opacity-100 group-hover:pointer-events-auto group-hover:translate-x-3 group-hover:opacity-100 hover:bg-destructive/15 hover:text-destructive"
            disabled={disabled}
            onClick={onRemove}
            size="icon"
            type="button"
            variant="ghost"
          >
            <Trash2 className="h-4 w-4" />
          </Button>
        ) : null}
      </div>

      <div
        className="group relative flex h-18 items-center justify-center"
        data-menu-open={addMenuOpen}
        data-terminal={terminal}
        data-testid="workflow-node-ingress"
      >
        {terminal ? null : (
          <ArrowDown
            aria-hidden="true"
            className="h-5 w-5 text-muted-foreground transition-opacity duration-200 ease-out group-hover:opacity-10 group-data-[menu-open=true]:opacity-10 group-has-[:focus-visible]:opacity-10 motion-reduce:transition-none"
          />
        )}
        <DropdownMenu onOpenChange={setAddMenuOpen}>
          <DropdownMenuTrigger asChild>
            <Button
              aria-label={
                title === "Trigger" ? "Add step" : `Add after ${title}`
              }
              className={cn(
                "relative z-10 h-7 w-7 rounded-full bg-background shadow-sm",
                !terminal &&
                  "pointer-events-none absolute scale-125 opacity-0 transition-[opacity,transform,background-color,color,border-color,box-shadow] duration-200 ease-out group-hover:pointer-events-auto group-hover:scale-100 group-hover:opacity-100 group-data-[menu-open=true]:pointer-events-auto group-data-[menu-open=true]:scale-100 group-data-[menu-open=true]:opacity-100 focus-visible:pointer-events-auto focus-visible:scale-100 focus-visible:opacity-100 motion-reduce:transition-none",
              )}
              disabled={disabled}
              size="icon"
              type="button"
              variant="outline"
            >
              <Plus className="h-3.5 w-3.5" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="center" side="right" sideOffset={8}>
            {SELECTABLE_ACTION_TYPES.map((action) => (
              <DropdownMenuItem
                key={action}
                onSelect={() => onAddAfter(action)}
              >
                <span>{ACTION_LABELS[action]}</span>
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </li>
  );
}

export const WorkflowFormBuilder = React.forwardRef<
  WorkflowFormBuilderHandle,
  WorkflowFormBuilderProps
>(function WorkflowFormBuilder(
  {
    channels: _channels,
    disabled,
    nameLeadingContainer,
    mode,
    onChange,
    onSelectedNodeChange,
    parseError,
    scopeField,
    selectedNode: selectedRouteNode,
    workflowChannelId,
    yaml,
  },
  ref,
) {
  // Parse once on mount instead of calling yamlToFormState three times
  const initialParseRef = React.useRef(yaml ? yamlToFormState(yaml) : null);
  const [formState, setFormState] = React.useState<WorkflowFormState>(
    initialParseRef.current?.ok
      ? initialParseRef.current.state
      : DEFAULT_FORM_STATE,
  );
  const selectedNode =
    selectedRouteNode?.type === "trigger" ||
    (selectedRouteNode?.type === "step" &&
      formState.steps.some((step) => step.id === selectedRouteNode.stepId))
      ? selectedRouteNode
      : null;
  const [selectionDirection, setSelectionDirection] = React.useState<1 | -1>(1);
  const [narrowInspector, setNarrowInspector] = React.useState(false);
  const containerRef = React.useRef<HTMLDivElement>(null);
  const shouldReduceMotion = useReducedMotion();
  const previousModeRef = React.useRef(mode);
  const lastSynchronizedYamlRef = React.useRef(yaml);
  const canonicalYamlRef = React.useRef(yaml);
  const pendingPaneReconciliationRef = React.useRef<WorkflowEditorPane>(null);

  React.useLayoutEffect(() => {
    const container = containerRef.current;
    if (!container || typeof ResizeObserver === "undefined") return;
    const update = () => setNarrowInspector(container.clientWidth <= 58 * 16);
    update();
    const observer = new ResizeObserver(update);
    observer.observe(container);
    return () => observer.disconnect();
  }, []);

  const updateFormState = React.useCallback(
    (next: WorkflowFormState) => {
      const nextYaml = formStateToYaml(next);
      lastSynchronizedYamlRef.current = nextYaml;
      canonicalYamlRef.current = nextYaml;
      setFormState(next);
      onChange(nextYaml);
    },
    [onChange],
  );

  React.useEffect(() => {
    if (previousModeRef.current === mode) return;
    previousModeRef.current = mode;

    if (mode === "yaml") {
      onSelectedNodeChange(null);
      return;
    }

    const result = yamlToFormState(yaml);
    if (result.ok) {
      setFormState(result.state);
      lastSynchronizedYamlRef.current = yaml;
    }
  }, [mode, onSelectedNodeChange, yaml]);

  React.useLayoutEffect(() => {
    canonicalYamlRef.current = yaml;
    if (mode !== "form" || yaml === lastSynchronizedYamlRef.current) return;
    const result = yamlToFormState(yaml);
    if (result.ok) {
      setFormState(result.state);
      lastSynchronizedYamlRef.current = yaml;
      return;
    }

    // The header can rename or disable a definition whose body is still
    // incomplete — a step that has no message text yet fails form validation.
    // Adopt those fields anyway, otherwise the next form edit re-serializes the
    // values this state was holding before the header wrote them.
    const header = readWorkflowDocumentFields(yaml);
    if (!header.editable) return;
    lastSynchronizedYamlRef.current = yaml;
    setFormState((current) => {
      const name = header.name ?? current.name;
      const enabled = header.enabled !== false;
      return name === current.name && enabled === current.enabled
        ? current
        : { ...current, enabled, name };
    });
  }, [mode, yaml]);

  React.useEffect(() => {
    if (pendingPaneReconciliationRef.current) {
      if (
        selectedRouteNode?.type === pendingPaneReconciliationRef.current.type &&
        (selectedRouteNode?.type !== "step" ||
          (pendingPaneReconciliationRef.current.type === "step" &&
            selectedRouteNode.stepId ===
              pendingPaneReconciliationRef.current.stepId))
      ) {
        pendingPaneReconciliationRef.current = null;
      }
      return;
    }
    if (
      mode === "form" &&
      selectedRouteNode?.type === "step" &&
      !formState.steps.some((step) => step.id === selectedRouteNode.stepId)
    ) {
      onSelectedNodeChange(null);
    }
  }, [formState.steps, mode, onSelectedNodeChange, selectedRouteNode]);

  const selectNode = React.useCallback(
    (nextNode: Exclude<WorkflowEditorPane, null>) => {
      if (selectedNode) {
        const currentPosition = nodePosition(selectedNode, formState.steps);
        const nextPosition = nodePosition(nextNode, formState.steps);
        if (nextPosition !== currentPosition) {
          setSelectionDirection(nextPosition < currentPosition ? -1 : 1);
        }
      }
      onSelectedNodeChange(nextNode);
    },
    [formState.steps, onSelectedNodeChange, selectedNode],
  );

  const insertStep = React.useCallback(
    (index: number, action: ActionType) => {
      const synchronizedState = yamlToFormState(canonicalYamlRef.current);
      const sourceState = synchronizedState.ok
        ? synchronizedState.state
        : formState;
      const nextSteps = [...sourceState.steps];
      const newStep: StepFormState = {
        id: nextStepId(sourceState.steps),
        action,
      };
      if (action === "call_webhook") {
        newStep.method = "POST";
      }
      nextSteps.splice(index, 0, newStep);
      updateFormState({
        ...sourceState,
        steps: nextSteps,
      });
      selectNode({ type: "step", stepId: newStep.id });
    },
    [formState, selectNode, updateFormState],
  );

  React.useImperativeHandle(
    ref,
    () => ({
      addFirstStep: () => insertStep(0, "send_message"),
      closeInspector: () => {
        if (!selectedNode) return false;
        onSelectedNodeChange(null);
        return true;
      },
      synchronizeYaml: (nextYaml: string) => {
        const result = yamlToFormState(nextYaml);
        if (!result.ok) return;
        lastSynchronizedYamlRef.current = nextYaml;
        canonicalYamlRef.current = nextYaml;
        setFormState(result.state);
      },
    }),
    [insertStep, onSelectedNodeChange, selectedNode],
  );

  const removeStep = React.useCallback(
    (index: number) => {
      updateFormState({
        ...formState,
        steps: formState.steps.filter((_, i) => i !== index),
      });

      if (selectedNode?.type !== "step") return;
      const selectedIndex = formState.steps.findIndex(
        (step) => step.id === selectedNode.stepId,
      );

      if (selectedIndex === index) {
        const fallbackPane =
          index > 0
            ? { type: "step" as const, stepId: formState.steps[index - 1].id }
            : formState.steps[index + 1]
              ? {
                  type: "step" as const,
                  stepId: formState.steps[index + 1].id,
                }
              : ({ type: "trigger" } as const);
        pendingPaneReconciliationRef.current = fallbackPane;
        setSelectionDirection(-1);
        onSelectedNodeChange(fallbackPane);
      }
    },
    [formState, onSelectedNodeChange, selectedNode, updateFormState],
  );

  const updateStep = React.useCallback(
    (index: number, step: StepFormState) => {
      const previousStep = formState.steps[index];
      const next = [...formState.steps];
      next[index] = step;
      updateFormState({ ...formState, steps: next });
      if (
        selectedNode?.type === "step" &&
        previousStep?.id === selectedNode.stepId &&
        step.id !== previousStep.id
      ) {
        onSelectedNodeChange({ type: "step", stepId: step.id });
      }
    },
    [formState, onSelectedNodeChange, selectedNode, updateFormState],
  );

  const selectedStep =
    selectedNode?.type === "step"
      ? formState.steps.find((step) => step.id === selectedNode.stepId)
      : undefined;
  const selectedStepIndex = selectedStep
    ? formState.steps.findIndex((step) => step.id === selectedStep.id)
    : -1;

  return (
    <>
      <div
        className="flex h-full min-h-0 flex-col [container-type:inline-size]"
        ref={containerRef}
      >
        {parseError ? (
          <p
            aria-live="assertive"
            className="mx-6 mt-3 rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive"
            role="alert"
          >
            Cannot switch to form view: {parseError}
          </p>
        ) : null}

        {mode === "yaml" ? (
          <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-hidden px-6 pb-3 pt-3">
            <div className="max-w-md flex-shrink-0">{scopeField}</div>
            <div className="flex min-h-0 flex-1 flex-col gap-1.5">
              <Textarea
                aria-label="Workflow YAML"
                autoCapitalize="off"
                className="min-h-0 flex-1 resize-none font-mono text-xs"
                disabled={disabled}
                onChange={(event) => onChange(event.target.value)}
                value={yaml}
              />
              <p className="flex-shrink-0 text-xs text-muted-foreground">
                Edit the raw YAML definition directly.
              </p>
            </div>
          </div>
        ) : (
          <div className="flex min-h-0 flex-1 flex-col">
            <div className="relative isolate flex min-h-0 flex-1">
              <div className="min-h-0 min-w-0 flex-1 overflow-y-auto px-6 py-5">
                <div className="mx-auto w-full max-w-sm">
                  {scopeField ? <div className="mb-3">{scopeField}</div> : null}
                  <ol aria-label="Workflow sequence">
                    <WorkflowNode
                      description={TRIGGER_LABELS[formState.trigger.on]}
                      disabled={disabled}
                      icon={<Zap className="h-4 w-4" />}
                      label={`Trigger: ${TRIGGER_LABELS[formState.trigger.on]}`}
                      onAddAfter={(action) => insertStep(0, action)}
                      onClick={() => selectNode({ type: "trigger" })}
                      selected={selectedNode?.type === "trigger"}
                      terminal={formState.steps.length === 0}
                      title="Trigger"
                    />

                    {formState.steps.map((step, index) => {
                      const stepName = step.name?.trim();
                      const actionLabel = ACTION_LABELS[step.action];
                      const nodeTitle = stepName || actionLabel;
                      return (
                        <WorkflowNode
                          description={nodeTitle}
                          disabled={disabled}
                          key={step.id}
                          label={`Step ${index + 1}: ${nodeTitle}`}
                          number={index + 1}
                          onAddAfter={(action) => insertStep(index + 1, action)}
                          onClick={() =>
                            selectNode({ type: "step", stepId: step.id })
                          }
                          onRemove={() => removeStep(index)}
                          selected={
                            selectedNode?.type === "step" &&
                            selectedNode.stepId === step.id
                          }
                          showTitle={false}
                          subtitle={stepName ? actionLabel : undefined}
                          terminal={index === formState.steps.length - 1}
                          title={`Step ${index + 1}`}
                        />
                      );
                    })}
                  </ol>
                </div>
              </div>

              <AnimatePresence>
                {selectedNode ? (
                  <motion.button
                    animate={{ opacity: 1 }}
                    aria-label="Close inspector overlay"
                    className="absolute inset-0 z-20 hidden bg-background/15 backdrop-blur-sm [@container(max-width:58rem)]:block"
                    data-testid="workflow-node-inspector-backdrop"
                    exit={{ opacity: 0 }}
                    initial={{ opacity: 0 }}
                    key="workflow-node-inspector-backdrop"
                    onClick={() => onSelectedNodeChange(null)}
                    transition={
                      shouldReduceMotion
                        ? { duration: 0 }
                        : { duration: 0.18, ease: "easeOut" }
                    }
                    type="button"
                  />
                ) : null}
                {selectedNode ? (
                  <FocusScope
                    asChild
                    loop={narrowInspector}
                    trapped={narrowInspector}
                  >
                    <motion.aside
                      animate={{ opacity: 1, width: "26rem", x: 0 }}
                      aria-label={
                        narrowInspector ? "Workflow node inspector" : undefined
                      }
                      aria-modal={narrowInspector || undefined}
                      className="flex flex-shrink-0 p-4 [@container(max-width:58rem)]:absolute [@container(max-width:58rem)]:inset-y-0 [@container(max-width:58rem)]:right-0 [@container(max-width:58rem)]:z-30 [@container(max-width:58rem)]:max-w-full"
                      data-testid="workflow-node-inspector"
                      exit={{ opacity: 0, width: 0, x: 24 }}
                      initial={{ opacity: 0, width: 0, x: 24 }}
                      key="workflow-node-inspector"
                      onKeyDown={(event) => {
                        if (event.key !== "Escape" || !narrowInspector) return;
                        event.preventDefault();
                        event.stopPropagation();
                        onSelectedNodeChange(null);
                      }}
                      role={narrowInspector ? "dialog" : "complementary"}
                      transition={
                        shouldReduceMotion
                          ? { duration: 0 }
                          : { duration: 0.24, ease: [0.22, 1, 0.36, 1] }
                      }
                    >
                      <div className="relative flex min-w-0 flex-1 flex-col overflow-hidden rounded-xl bg-muted/40 [@container(max-width:58rem)]:bg-background [@container(max-width:58rem)]:shadow-2xl">
                        <div
                          aria-hidden="true"
                          className="pointer-events-none absolute inset-0 z-0 hidden bg-muted/40 [@container(max-width:58rem)]:block"
                        />
                        <div className="relative z-10 flex w-96 min-w-96 flex-shrink-0 items-start justify-between gap-3 px-5 pb-3 pt-5 [@container(max-width:26rem)]:w-full [@container(max-width:26rem)]:min-w-0">
                          <div className="min-w-0">
                            <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                              {selectedNode.type === "trigger"
                                ? "Trigger"
                                : `Step ${selectedStepIndex + 1}`}
                            </p>
                            {selectedNode.type === "trigger" ? (
                              <InspectorTypeMenu
                                ariaLabel="Trigger event"
                                disabled={disabled}
                                labels={TRIGGER_LABELS}
                                onChange={(triggerType) => {
                                  const next = withTriggerType(
                                    formState,
                                    triggerType,
                                  );
                                  updateFormState({
                                    ...next,
                                    steps: supportsMessageTextCondition(
                                      triggerType,
                                    )
                                      ? next.steps
                                      : next.steps.map((step) => ({
                                          ...step,
                                          condition: undefined,
                                        })),
                                    trigger:
                                      triggerType === "schedule"
                                        ? defaultScheduleTrigger()
                                        : next.trigger,
                                  });
                                }}
                                options={SELECTABLE_TRIGGER_TYPES}
                                value={formState.trigger.on}
                              />
                            ) : selectedStep ? (
                              <InspectorTypeMenu
                                ariaLabel="Action"
                                disabled={disabled}
                                labels={ACTION_LABELS}
                                onChange={(action) => {
                                  const next = { ...selectedStep, action };
                                  if (
                                    action === "call_webhook" &&
                                    !next.method
                                  ) {
                                    next.method = "POST";
                                  }
                                  updateStep(selectedStepIndex, next);
                                }}
                                options={SELECTABLE_ACTION_TYPES}
                                value={selectedStep.action}
                              />
                            ) : null}
                          </div>
                          <div className="flex items-center gap-1">
                            {selectedNode.type === "step" && selectedStep ? (
                              <Button
                                aria-label="Remove step"
                                className="h-8 w-8"
                                disabled={disabled}
                                onClick={() => removeStep(selectedStepIndex)}
                                size="icon"
                                type="button"
                                variant="ghost"
                              >
                                <Trash2 className="h-4 w-4 text-muted-foreground" />
                              </Button>
                            ) : null}
                            <Button
                              aria-label="Close inspector"
                              className="h-8 w-8"
                              onClick={() => onSelectedNodeChange(null)}
                              size="icon"
                              type="button"
                              variant="ghost"
                            >
                              <X className="h-4 w-4" />
                            </Button>
                          </div>
                        </div>

                        <div className="relative z-10 min-h-0 w-96 min-w-96 flex-1 overflow-y-auto px-5 pb-5 pt-2 [@container(max-width:26rem)]:w-full [@container(max-width:26rem)]:min-w-0">
                          <AnimatePresence
                            custom={selectionDirection}
                            initial={false}
                            mode="wait"
                          >
                            <motion.div
                              animate="center"
                              custom={selectionDirection}
                              exit="exit"
                              initial="enter"
                              key={
                                selectedNode.type === "trigger"
                                  ? "trigger"
                                  : `step-${selectedNode.stepId}`
                              }
                              transition={
                                shouldReduceMotion
                                  ? { duration: 0 }
                                  : { duration: 0.15, ease: "easeOut" }
                              }
                              variants={inspectorContentVariants}
                            >
                              {selectedNode.type === "trigger" ? (
                                <div>
                                  <TriggerConfigFields
                                    disabled={disabled}
                                    onUpdate={(trigger) =>
                                      updateFormState({ ...formState, trigger })
                                    }
                                    trigger={formState.trigger}
                                  />
                                </div>
                              ) : selectedStep ? (
                                <WorkflowStepCard
                                  bare
                                  disabled={disabled}
                                  index={selectedStepIndex}
                                  onRemove={() => removeStep(selectedStepIndex)}
                                  onUpdate={(updated) =>
                                    updateStep(selectedStepIndex, updated)
                                  }
                                  showHeader={false}
                                  step={selectedStep}
                                  triggerType={formState.trigger.on}
                                  workflowChannelId={workflowChannelId}
                                />
                              ) : null}
                            </motion.div>
                          </AnimatePresence>
                        </div>
                      </div>
                    </motion.aside>
                  </FocusScope>
                ) : null}
              </AnimatePresence>
            </div>
          </div>
        )}
      </div>
      {mode === "form" && nameLeadingContainer
        ? createPortal(
            <Switch
              aria-label="Enable workflow"
              checked={formState.enabled}
              disabled={disabled}
              id="wf-enabled"
              onCheckedChange={(checked) =>
                updateFormState({ ...formState, enabled: checked })
              }
            />,
            nameLeadingContainer,
          )
        : null}
    </>
  );
});
