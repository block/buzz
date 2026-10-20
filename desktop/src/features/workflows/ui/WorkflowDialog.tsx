import * as React from "react";
import { Check, Code, Pencil, X } from "lucide-react";
import { useBlocker } from "@tanstack/react-router";
import { stringify as yamlStringify } from "yaml";

import {
  useCreateWorkflowMutation,
  useUpdateWorkflowMutation,
} from "@/features/workflows/hooks";
import { generateBackupPassphrase } from "@/shared/api/tauriIdentity";
import type { Channel, Workflow } from "@/shared/api/types";
import { getRelayHttpUrl } from "@/shared/api/tauri";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Popover, PopoverContent } from "@/shared/ui/popover";
import { ChannelCombobox } from "./ChannelCombobox";
import { WorkflowActionsMenu } from "./WorkflowActionsMenu";
import { WorkflowDetailPanel } from "./WorkflowDetailPanel";
import {
  WorkflowFormBuilder,
  type WorkflowEditorMode,
  type WorkflowFormBuilderHandle,
} from "./WorkflowFormBuilder";
import { WorkflowWebhookSecretDialog } from "./WorkflowWebhookSecretDialog";
import { getWorkflowActivationWarning } from "./workflowActivationWarning";
import { getWorkflowEnabled } from "./workflowDefinition";
import type { WorkflowEditorPane } from "./workflowEditorPane";
import {
  DEFAULT_FORM_STATE,
  formStateToYaml,
  yamlToFormState,
} from "./workflowFormTypes";
import {
  readWorkflowDocumentFields,
  readWorkflowHeaderState,
  yamlWithWorkflowEnabled,
  yamlWithWorkflowName,
} from "./workflowYamlDocument";

type DialogMode = "create" | "edit" | "duplicate";

type WorkflowDialogProps = {
  channels: Channel[];
  initialChannelId?: string;
  mode: DialogMode;
  onDeleteWorkflow: (workflow: Workflow) => void;
  onDuplicateWorkflow: (workflowId: string) => void;
  onEditWorkflow: (workflowId: string) => void;
  onEditorPaneChange: (pane: WorkflowEditorPane) => void;
  onOpenChange: (open: boolean) => void;
  onTriggerWorkflow: (workflowId: string) => void;
  open: boolean;
  pane: WorkflowEditorPane;
  workflow?: Workflow | null;
};

function getInitialYaml(
  mode: DialogMode,
  workflow: Workflow | null | undefined,
): string {
  if (!workflow) return "";
  const def = { ...workflow.definition };
  if (mode === "duplicate") {
    def.name = `${def.name ?? workflow.name} (copy)`;
  }
  return yamlStringify(def);
}

function getInitialEditorMode(yaml: string): WorkflowEditorMode {
  if (!yaml) return "form";
  return yamlToFormState(yaml).ok ? "form" : "yaml";
}

const TITLES: Record<DialogMode, string> = {
  create: "Create workflow",
  edit: "Edit workflow",
  duplicate: "Duplicate workflow",
};

const SUBMIT_LABELS: Record<DialogMode, string> = {
  create: "Create workflow",
  edit: "Save changes",
  duplicate: "Create copy",
};

const PENDING_LABELS: Record<DialogMode, string> = {
  create: "Creating…",
  edit: "Saving…",
  duplicate: "Creating…",
};

function WorkflowNameEditor({
  disabled,
  generating,
  name,
  onCommit,
  onEditingChange,
}: {
  disabled: boolean;
  generating: boolean;
  name: string;
  onCommit: (name: string) => boolean;
  onEditingChange?: (editing: boolean) => void;
}) {
  const [editing, setEditing] = React.useState(false);
  const [draft, setDraft] = React.useState(name);
  const inputRef = React.useRef<HTMLInputElement>(null);

  React.useEffect(() => {
    if (!editing) setDraft(name);
  }, [editing, name]);

  React.useEffect(() => {
    if (editing) inputRef.current?.select();
  }, [editing]);

  const changeEditing = React.useCallback(
    (nextEditing: boolean) => {
      setEditing(nextEditing);
      onEditingChange?.(nextEditing);
    },
    [onEditingChange],
  );

  const commit = React.useCallback(() => {
    const nextName = inputRef.current?.value.trim() ?? draft.trim();
    if (!nextName || !onCommit(nextName)) return;
    changeEditing(false);
  }, [changeEditing, draft, onCommit]);

  if (editing) {
    return (
      <div className="flex h-6 items-center gap-1.5">
        <Input
          aria-label="Workflow name"
          autoCapitalize="off"
          autoCorrect="off"
          className="h-6 w-72 px-2 font-mono text-sm"
          disabled={disabled}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              commit();
            } else if (event.key === "Escape") {
              event.preventDefault();
              setDraft(name);
              changeEditing(false);
            }
          }}
          ref={inputRef}
          defaultValue={name}
        />
        <Button
          aria-label="Save workflow name"
          className="text-muted-foreground"
          disabled={disabled || !draft.trim()}
          onClick={commit}
          onPointerDown={(event) => {
            // Commit before pointer focus changes can re-render the portalled
            // header controls and restore the generated name.
            event.preventDefault();
            commit();
          }}
          size="icon-xs"
          title="Save workflow name"
          type="button"
          variant="ghost"
        >
          <Check />
        </Button>
      </div>
    );
  }

  return (
    <div className="inline-flex h-6 max-w-full min-w-0 items-center gap-1.5 font-mono text-sm text-muted-foreground">
      <span className="min-w-0 truncate">
        {generating ? "Generating name…" : name || "Untitled workflow"}
      </span>
      <Button
        aria-label="Edit workflow name"
        className="text-muted-foreground [&_svg]:size-3"
        disabled={disabled || generating}
        onClick={() => changeEditing(true)}
        size="icon-xs"
        title="Edit workflow name"
        type="button"
        variant="ghost"
      >
        <Pencil />
      </Button>
    </div>
  );
}

export function WorkflowDialog({
  channels,
  initialChannelId,
  mode,
  onDeleteWorkflow,
  onDuplicateWorkflow,
  onEditWorkflow,
  onEditorPaneChange,
  onOpenChange,
  onTriggerWorkflow,
  open,
  pane,
  workflow,
}: WorkflowDialogProps) {
  const formBuilderRef = React.useRef<WorkflowFormBuilderHandle>(null);
  const workflowSnapshotRef = React.useRef(workflow);
  const workflowSnapshot = workflowSnapshotRef.current;
  const channelId =
    mode === "edit" && workflowSnapshot?.channelId
      ? workflowSnapshot.channelId
      : mode === "create" &&
          initialChannelId &&
          channels.some((channel) => channel.id === initialChannelId)
        ? initialChannelId
        : "";

  const [selectedChannelId, setSelectedChannelId] = React.useState(channelId);
  const [yamlDefinition, setYamlDefinition] = React.useState(() =>
    getInitialYaml(mode, workflowSnapshot),
  );
  const [editorMode, setEditorMode] = React.useState<WorkflowEditorMode>(() =>
    getInitialEditorMode(getInitialYaml(mode, workflowSnapshot)),
  );
  const [editorParseError, setEditorParseError] = React.useState<string | null>(
    null,
  );
  const [historyOpen, setHistoryOpen] = React.useState(false);
  const [channelAutoOpenPending, setChannelAutoOpenPending] = React.useState(
    mode === "create" && !channelId,
  );
  const [savedWebhookInfo, setSavedWebhookInfo] = React.useState<{
    relayHttpUrl: string | null;
    relayUrlError: string | null;
    webhookSecret: string;
    workflowId: string;
  } | null>(null);
  const [discardConfirmationOpen, setDiscardConfirmationOpen] =
    React.useState(false);
  const [activationConfirmationOpen, setActivationConfirmationOpen] =
    React.useState(false);
  const [pendingCreateYaml, setPendingCreateYaml] = React.useState<
    string | null
  >(null);
  const [formValid, setFormValid] = React.useState(true);
  const [secretConfirmationOpen, setSecretConfirmationOpen] =
    React.useState(false);
  const [generatingName, setGeneratingName] = React.useState(false);
  const initialValuesRef = React.useRef({
    channelId,
    yaml: getInitialYaml(mode, workflowSnapshot),
  });
  const yamlDefinitionRef = React.useRef(yamlDefinition);
  const allowNavigationRef = React.useRef(false);
  const proceedingNavigationRef = React.useRef(false);
  const pendingEditorTransitionRef = React.useRef<(() => void) | null>(null);

  const createMutation = useCreateWorkflowMutation(selectedChannelId);
  const updateMutation = useUpdateWorkflowMutation(
    workflowSnapshot?.id ?? "",
    workflowSnapshot?.revision ?? "",
  );
  const mutation = mode === "edit" ? updateMutation : createMutation;

  const selectedChannel =
    channels.find((c) => c.id === selectedChannelId) ?? null;
  const parsedDefinition = yamlDefinition.trim()
    ? yamlToFormState(yamlDefinition)
    : null;
  const isAddingFirstStep =
    mode === "create" &&
    editorMode === "form" &&
    (parsedDefinition === null ||
      (parsedDefinition.ok && parsedDefinition.state.steps.length === 0));

  const resetCreate = createMutation.reset;
  const resetUpdate = updateMutation.reset;

  React.useEffect(() => {
    let active = true;
    setSavedWebhookInfo(null);
    setDiscardConfirmationOpen(false);
    setActivationConfirmationOpen(false);
    setPendingCreateYaml(null);
    setFormValid(true);
    resetCreate();
    resetUpdate();

    if (mode === "create" && !workflowSnapshot) {
      setGeneratingName(true);
      void generateBackupPassphrase({ words: 3, separator: "-" })
        .then((name) => {
          if (!active || yamlDefinitionRef.current.trim()) return;
          const generatedYaml = formStateToYaml({
            ...DEFAULT_FORM_STATE,
            name,
          });
          yamlDefinitionRef.current = generatedYaml;
          initialValuesRef.current = {
            ...initialValuesRef.current,
            yaml: generatedYaml,
          };
          setYamlDefinition(generatedYaml);
          formBuilderRef.current?.synchronizeYaml(generatedYaml);
        })
        .catch(() => {
          // Leave the editable "Untitled workflow" fallback in place.
        })
        .finally(() => {
          if (active) setGeneratingName(false);
        });
    } else {
      setGeneratingName(false);
    }

    return () => {
      active = false;
    };
  }, [mode, resetCreate, resetUpdate, workflowSnapshot]);

  const closeDialog = React.useCallback(() => {
    resetCreate();
    resetUpdate();
    setDiscardConfirmationOpen(false);
    setActivationConfirmationOpen(false);
    setPendingCreateYaml(null);
    onOpenChange(false);
  }, [onOpenChange, resetCreate, resetUpdate]);

  const isDirty =
    yamlDefinition !== initialValuesRef.current.yaml ||
    selectedChannelId !== initialValuesRef.current.channelId;
  const navigationBlocker = useBlocker({
    enableBeforeUnload: isDirty || savedWebhookInfo !== null,
    shouldBlockFn: ({ current, next }) => {
      const currentSearch = current.search as {
        pane?: unknown;
        view?: unknown;
      };
      const nextSearch = next.search as { pane?: unknown; view?: unknown };
      const isPaneOnlyNavigation =
        current.pathname === next.pathname &&
        currentSearch.view === nextSearch.view &&
        currentSearch.pane !== nextSearch.pane;
      return (
        (isDirty || savedWebhookInfo !== null) &&
        !allowNavigationRef.current &&
        !isPaneOnlyNavigation
      );
    },
    withResolver: true,
  });

  React.useEffect(() => {
    if (navigationBlocker.status === "blocked") {
      if (savedWebhookInfo) {
        setSecretConfirmationOpen(true);
      } else {
        setDiscardConfirmationOpen(true);
      }
    }
  }, [navigationBlocker.status, savedWebhookInfo]);

  const requestEditorTransition = React.useCallback(
    (transition: () => void) => {
      if (isDirty) {
        pendingEditorTransitionRef.current = transition;
        setDiscardConfirmationOpen(true);
        return;
      }
      transition();
    },
    [isDirty],
  );

  const handleOpenChange = React.useCallback(
    (nextOpen: boolean) => {
      if (nextOpen) {
        onOpenChange(true);
      } else if (savedWebhookInfo) {
        setSecretConfirmationOpen(true);
      } else if (isDirty) {
        setDiscardConfirmationOpen(true);
      } else {
        closeDialog();
      }
    },
    [closeDialog, isDirty, onOpenChange, savedWebhookInfo],
  );

  async function saveWorkflow(yaml: string) {
    try {
      const saved = await mutation.mutateAsync(yaml);
      initialValuesRef.current = {
        channelId: selectedChannelId,
        yaml,
      };
      if (saved.webhookSecret) {
        allowNavigationRef.current = false;
        const webhookInfo = {
          relayHttpUrl: null,
          relayUrlError: null,
          webhookSecret: saved.webhookSecret,
          workflowId: saved.workflow.id,
        };
        setSavedWebhookInfo(webhookInfo);
        try {
          const relayHttpUrl = await getRelayHttpUrl();
          setSavedWebhookInfo({ ...webhookInfo, relayHttpUrl });
        } catch (error) {
          setSavedWebhookInfo({
            ...webhookInfo,
            relayUrlError:
              error instanceof Error
                ? error.message
                : "Could not load the webhook URL",
          });
        }
      } else {
        allowNavigationRef.current = true;
        closeDialog();
      }
    } catch {
      // React Query stores the error; keep the dialog open and dirty.
    }
  }

  function handleSubmit() {
    if (!selectedChannelId || !yamlDefinition.trim() || !formValid) return;

    const documentEnabled = readWorkflowDocumentFields(yamlDefinition).enabled;
    const savedEnabled = workflowSnapshot
      ? getWorkflowEnabled(workflowSnapshot.definition)
      : null;
    const enablesWorkflow =
      documentEnabled !== false && (mode !== "edit" || savedEnabled === false);
    if (enablesWorkflow && getWorkflowActivationWarning(yamlDefinition)) {
      const disabledYaml = yamlWithWorkflowEnabled(yamlDefinition, false);
      if (disabledYaml === null) return;
      setPendingCreateYaml(disabledYaml);
      setActivationConfirmationOpen(true);
      return;
    }

    void saveWorkflow(yamlDefinition);
  }

  function handleCreateActivation(enabled: boolean) {
    if (pendingCreateYaml === null) return;
    const yaml = enabled
      ? yamlWithWorkflowEnabled(pendingCreateYaml, true)
      : pendingCreateYaml;
    if (yaml === null) return;
    setActivationConfirmationOpen(false);
    setPendingCreateYaml(null);
    void saveWorkflow(yaml);
  }

  const handleEditorModeChange = React.useCallback(
    (nextMode: string) => {
      if (nextMode === editorMode) return;

      if (nextMode === "yaml") {
        setEditorParseError(null);
        setEditorMode("yaml");
        return;
      }

      if (!yamlDefinition.trim()) {
        setEditorParseError(null);
        setEditorMode("form");
        return;
      }

      const result = yamlToFormState(yamlDefinition);
      if (result.ok) {
        setEditorParseError(null);
        setEditorMode("form");
      } else {
        setEditorParseError(result.error);
      }
    },
    [editorMode, yamlDefinition],
  );

  // Header state reads the YAML document directly rather than the fully
  // validated form state: a step that is still being filled in (a new
  // send_message with no text yet) fails form validation, and gating the name
  // on that made the title blank out as soon as a step pane opened.
  const {
    canEdit: canEditWorkflowName,
    enabled: workflowEnabled,
    name: workflowName,
  } = readWorkflowHeaderState(yamlDefinition, {
    enabled: workflowSnapshot
      ? getWorkflowEnabled(workflowSnapshot.definition)
      : true,
    name: workflowSnapshot?.name,
  });
  const handleWorkflowNameCommit = React.useCallback(
    (name: string) => {
      const nextYaml = yamlWithWorkflowName(yamlDefinitionRef.current, name);
      if (nextYaml === null) return false;
      mutation.reset();
      yamlDefinitionRef.current = nextYaml;
      setYamlDefinition(nextYaml);
      return true;
    },
    [mutation.reset],
  );
  const handleToggleWorkflowEnabled = React.useCallback(() => {
    const nextYaml = yamlWithWorkflowEnabled(
      yamlDefinitionRef.current,
      !workflowEnabled,
    );
    if (nextYaml === null) return;
    mutation.reset();
    yamlDefinitionRef.current = nextYaml;
    setYamlDefinition(nextYaml);
  }, [mutation.reset, workflowEnabled]);
  const showChannelSelector = mode !== "edit";
  const activationWarning =
    pendingCreateYaml === null
      ? null
      : getWorkflowActivationWarning(pendingCreateYaml);

  return (
    <>
      <Dialog
        onOpenChange={handleOpenChange}
        open={open && savedWebhookInfo === null}
      >
        <DialogContent
          className="flex h-[88vh] max-h-[88vh] w-[calc(100vw-2rem)] max-w-6xl flex-col gap-0 overflow-hidden p-0"
          onEscapeKeyDown={(event) => {
            if (
              event.target instanceof HTMLElement &&
              event.target.closest("[data-workflow-filter-picker-search]")
            ) {
              event.preventDefault();
              return;
            }
            if (formBuilderRef.current?.closeInspector()) {
              event.preventDefault();
              event.stopPropagation();
            }
          }}
          showCloseButton={false}
        >
          <DialogHeader className="flex flex-shrink-0 flex-row items-center justify-between gap-6 space-y-0 px-6 pt-3 pb-2 text-left">
            <div className="space-y-0">
              <DialogTitle className="text-lg leading-tight">
                {TITLES[mode]}
              </DialogTitle>
              <DialogDescription className="sr-only">
                {mode === "edit"
                  ? "Update when this workflow runs and what it does."
                  : mode === "duplicate"
                    ? "Copy this workflow and adjust its details."
                    : "Automate actions when something happens in a channel."}
              </DialogDescription>
              <div className="flex items-center gap-2">
                <WorkflowNameEditor
                  disabled={mutation.isPending || !canEditWorkflowName}
                  generating={generatingName}
                  name={workflowName}
                  onCommit={handleWorkflowNameCommit}
                />
              </div>
            </div>
            <div className="flex items-center gap-2">
              {mode === "edit" && workflowSnapshot ? (
                <>
                  <Popover onOpenChange={setHistoryOpen} open={historyOpen}>
                    {/* TODO(workflow-run-history-capability): Restore this
                    icon-only entry point after Desktop gates it on the active
                    relay's advertised NIP-11 capabilities.
                    <PopoverTrigger asChild>
                      <Button
                        aria-label="Run history"
                        className="h-8 w-8"
                        size="icon"
                        type="button"
                        variant={historyOpen ? "secondary" : "outline"}
                      >
                        <History className="h-4 w-4" />
                      </Button>
                    </PopoverTrigger>
                    */}
                    <PopoverContent
                      align="end"
                      aria-label="Run history"
                      className="flex max-h-[min(32rem,var(--radix-popover-content-available-height))] w-96 max-w-[calc(100vw-2rem)] flex-col overflow-hidden p-0"
                      data-testid="workflow-history-dropdown"
                      sideOffset={8}
                    >
                      <div className="flex-shrink-0 border-b px-5 py-3">
                        <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                          Workflow
                        </p>
                        <h3 className="text-base font-semibold">Run history</h3>
                      </div>
                      <div className="min-h-0 flex-1">
                        <WorkflowDetailPanel
                          showDefinition={false}
                          showHeader={false}
                          workflowId={workflowSnapshot.id}
                        />
                      </div>
                    </PopoverContent>
                  </Popover>
                  <WorkflowActionsMenu
                    isEnabled={workflowEnabled}
                    isTogglingEnabled={
                      mutation.isPending || !canEditWorkflowName
                    }
                    onDelete={() => onDeleteWorkflow(workflowSnapshot)}
                    onDuplicate={() =>
                      requestEditorTransition(() =>
                        onDuplicateWorkflow(workflowSnapshot.id),
                      )
                    }
                    onEdit={() =>
                      requestEditorTransition(() =>
                        onEditWorkflow(workflowSnapshot.id),
                      )
                    }
                    onToggleEnabled={handleToggleWorkflowEnabled}
                    onTrigger={() => onTriggerWorkflow(workflowSnapshot.id)}
                  />
                </>
              ) : null}
              <DialogClose asChild>
                <Button
                  aria-label="Close"
                  className="h-8 w-8 text-muted-foreground"
                  size="icon"
                  type="button"
                  variant="ghost"
                >
                  <X className="h-4 w-4" />
                </Button>
              </DialogClose>
            </div>
          </DialogHeader>

          <div className="min-h-0 flex-1">
            <WorkflowFormBuilder
              channels={channels}
              disabled={mutation.isPending}
              mode={editorMode}
              nameLeadingContainer={null}
              onChange={(yaml) => {
                mutation.reset();
                yamlDefinitionRef.current = yaml;
                setYamlDefinition(yaml);
              }}
              onSelectedNodeChange={onEditorPaneChange}
              onValidityChange={setFormValid}
              parseError={editorParseError}
              ref={formBuilderRef}
              scopeField={
                showChannelSelector ? (
                  <div className="space-y-1">
                    <ChannelCombobox
                      channels={channels}
                      defaultOpen={
                        channelAutoOpenPending &&
                        editorMode === "form" &&
                        !selectedChannelId
                      }
                      disabled={mutation.isPending}
                      id="wf-channel-select"
                      onAutoOpen={() => setChannelAutoOpenPending(false)}
                      onChange={(value) => {
                        mutation.reset();
                        setSelectedChannelId(value);
                        if (value) onEditorPaneChange({ type: "trigger" });
                      }}
                      required
                      variant={editorMode === "yaml" ? "field" : "header"}
                      value={selectedChannelId}
                    />
                    {channels.length === 0 ? (
                      <p className="text-center text-xs text-muted-foreground">
                        Join or create a channel before adding a workflow.
                      </p>
                    ) : null}
                  </div>
                ) : mode === "edit" && selectedChannel ? (
                  <ChannelCombobox
                    channels={channels}
                    id="wf-channel-select"
                    onChange={setSelectedChannelId}
                    readOnly
                    variant={editorMode === "yaml" ? "field" : "header"}
                    value={selectedChannel.id}
                  />
                ) : null
              }
              selectedNode={
                mode === "create" && !selectedChannelId ? null : pane
              }
              workflowChannelId={selectedChannelId || null}
              yaml={yamlDefinition}
            />
          </div>

          {mutation.error instanceof Error ? (
            <p
              aria-live="polite"
              className="mx-6 mb-3 rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
              role="alert"
            >
              {mutation.error.message}
            </p>
          ) : null}

          <div className="flex flex-shrink-0 items-center justify-between gap-4 px-6 pt-2 pb-4">
            <Tabs onValueChange={handleEditorModeChange} value={editorMode}>
              <TabsList aria-label="Workflow editor mode" className="h-8 p-0.5">
                <TabsTrigger
                  className="h-7 px-3 text-xs"
                  disabled={mutation.isPending}
                  value="form"
                >
                  Form
                </TabsTrigger>
                <TabsTrigger
                  className="h-7 gap-1.5 px-3 text-xs"
                  disabled={mutation.isPending}
                  value="yaml"
                >
                  <Code className="h-3.5 w-3.5" />
                  YAML
                </TabsTrigger>
              </TabsList>
            </Tabs>
            <div className="flex items-center gap-2">
              <Button
                onClick={() => handleOpenChange(false)}
                type="button"
                variant="outline"
              >
                Cancel
              </Button>
              {isAddingFirstStep ? (
                <Button
                  aria-label="Add first step"
                  data-testid="workflow-dialog-primary-action"
                  disabled={!selectedChannelId || mutation.isPending}
                  onClick={() => formBuilderRef.current?.addFirstStep()}
                  type="button"
                >
                  Add step
                </Button>
              ) : (
                <Button
                  data-testid="workflow-dialog-primary-action"
                  disabled={
                    !selectedChannelId ||
                    !yamlDefinition.trim() ||
                    !formValid ||
                    mutation.isPending
                  }
                  onClick={handleSubmit}
                  type="button"
                >
                  {mutation.isPending
                    ? PENDING_LABELS[mode]
                    : SUBMIT_LABELS[mode]}
                </Button>
              )}
            </div>
          </div>
        </DialogContent>
      </Dialog>

      <AlertDialog
        onOpenChange={(nextOpen) => {
          setActivationConfirmationOpen(nextOpen);
          if (!nextOpen) setPendingCreateYaml(null);
        }}
        open={activationConfirmationOpen}
      >
        <AlertDialogContent data-testid="workflow-activation-confirmation">
          <AlertDialogHeader>
            <AlertDialogTitle>
              {activationWarning?.title ?? "Turn on this workflow?"}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {activationWarning?.description ??
                "Turn it on to let it run immediately, or keep it off until you’re ready."}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel asChild>
              <Button type="button" variant="ghost">
                Back
              </Button>
            </AlertDialogCancel>
            <Button
              onClick={() => handleCreateActivation(false)}
              type="button"
              variant="outline"
            >
              Keep off
            </Button>
            <AlertDialogAction asChild>
              <Button
                onClick={() => handleCreateActivation(true)}
                type="button"
              >
                Turn on
              </Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog
        onOpenChange={(nextOpen) => {
          setDiscardConfirmationOpen(nextOpen);
          if (!nextOpen) {
            pendingEditorTransitionRef.current = null;
          }
          if (
            !nextOpen &&
            navigationBlocker.status === "blocked" &&
            !proceedingNavigationRef.current
          ) {
            navigationBlocker.reset();
          }
        }}
        open={discardConfirmationOpen}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Discard changes?</AlertDialogTitle>
            <AlertDialogDescription>
              Your unsaved workflow changes will be lost.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel asChild>
              <Button type="button" variant="outline">
                Keep editing
              </Button>
            </AlertDialogCancel>
            <AlertDialogAction asChild>
              <Button
                onClick={() => {
                  const pendingEditorTransition =
                    pendingEditorTransitionRef.current;
                  pendingEditorTransitionRef.current = null;
                  setDiscardConfirmationOpen(false);
                  if (pendingEditorTransition) {
                    allowNavigationRef.current = true;
                    pendingEditorTransition();
                    return;
                  }
                  if (navigationBlocker.status === "blocked") {
                    proceedingNavigationRef.current = true;
                    navigationBlocker.proceed();
                    return;
                  }
                  allowNavigationRef.current = true;
                  closeDialog();
                }}
                type="button"
                variant="destructive"
              >
                Discard changes
              </Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog
        onOpenChange={(nextOpen) => {
          setSecretConfirmationOpen(nextOpen);
          if (
            !nextOpen &&
            navigationBlocker.status === "blocked" &&
            !proceedingNavigationRef.current
          ) {
            navigationBlocker.reset();
          }
        }}
        open={secretConfirmationOpen}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Continue without this secret?</AlertDialogTitle>
            <AlertDialogDescription>
              This private webhook secret cannot be recovered. Copy and store it
              before continuing, or explicitly leave it behind.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel asChild>
              <Button type="button" variant="outline">
                Go back
              </Button>
            </AlertDialogCancel>
            <AlertDialogAction asChild>
              <Button
                onClick={() => {
                  setSecretConfirmationOpen(false);
                  allowNavigationRef.current = true;
                  if (navigationBlocker.status === "blocked") {
                    proceedingNavigationRef.current = true;
                    navigationBlocker.proceed();
                    return;
                  }
                  closeDialog();
                }}
                type="button"
                variant="destructive"
              >
                Continue
              </Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {savedWebhookInfo ? (
        <WorkflowWebhookSecretDialog
          onContinue={() => setSecretConfirmationOpen(true)}
          open
          relayHttpUrl={savedWebhookInfo.relayHttpUrl}
          relayUrlError={savedWebhookInfo.relayUrlError}
          webhookSecret={savedWebhookInfo.webhookSecret}
          workflowId={savedWebhookInfo.workflowId}
        />
      ) : null}
    </>
  );
}
