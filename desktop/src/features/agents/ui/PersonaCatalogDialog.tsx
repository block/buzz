import * as React from "react";
import { Plus, Upload } from "lucide-react";

import { isCatalogPersonaSelected } from "@/features/agents/lib/catalog";
import { isCatalogPersona } from "@/features/agents/lib/personaCatalogRelay";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { AgentPersona } from "@/shared/api/types";
import { useFeedbackToasts } from "@/shared/hooks/useToastEffect";
import { cn } from "@/shared/lib/cn";
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
import { Dialog } from "@/shared/ui/dialog";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Markdown } from "@/shared/ui/markdown";
import { Skeleton } from "@/shared/ui/skeleton";

import { AgentDefinitionMetadata } from "./AgentDefinitionMetadata";
import { PersonaAddedBy } from "./PersonaAddedBy";
import { personaCatalogCopy } from "./personaLibraryCopy";

type PersonaCatalogDialogProps = {
  createContent: (controls: {
    onDirtyChange: (dirty: boolean) => void;
    onRequestClose: () => void;
  }) => React.ReactNode;
  error: Error | null;
  feedbackErrorMessage: string | null;
  feedbackNoticeMessage: string | null;
  isLoading: boolean;
  isPending: boolean;
  onClearFeedback: () => void;
  onImportFile: (fileBytes: number[], fileName: string) => void;
  onOpenChange: (open: boolean) => void;
  onSelectPersona: (persona: AgentPersona, active: boolean) => void;
  open: boolean;
  personas: AgentPersona[];
};

type PendingNavigation =
  | { type: "close" }
  | { type: "selection"; selection: string };

const agentInstructionMarkdownClassName = [
  "mt-3 w-full min-w-0 max-w-full overflow-x-hidden leading-6 text-muted-foreground [&>*]:min-w-0 [&>*]:max-w-full [&_.code-block-lines]:min-w-0 [&_.code-block-lines]:max-w-full [&_.code-block-lines]:whitespace-pre-wrap [&_.code-block-lines]:[overflow-wrap:anywhere] [&_.inline-code-chip]:max-w-full [&_.inline-code-chip]:whitespace-pre-wrap [&_.inline-code-chip]:[overflow-wrap:anywhere] [&_blockquote]:!text-muted-foreground [&_code]:!text-muted-foreground [&_li]:text-muted-foreground [&_ol]:text-muted-foreground [&_p]:text-muted-foreground [&_strong]:text-muted-foreground [&_td]:text-muted-foreground [&_ul]:text-muted-foreground",
  "[&>h1]:!text-sm [&>h1]:!font-semibold [&>h1]:!leading-6 [&>h1]:!tracking-normal [&>h1]:!text-foreground",
  "[&>h2]:!text-sm [&>h2]:!font-semibold [&>h2]:!leading-6 [&>h2]:!tracking-normal [&>h2]:!text-foreground",
  "[&>h3]:!text-sm [&>h3]:!font-semibold [&>h3]:!leading-6 [&>h3]:!tracking-normal [&>h3]:!text-foreground",
  "[&>h4]:!text-sm [&>h4]:!font-semibold [&>h4]:!leading-6 [&>h4]:!tracking-normal [&>h4]:!text-foreground",
  "[&>h5]:!text-sm [&>h5]:!font-semibold [&>h5]:!leading-6 [&>h5]:!tracking-normal [&>h5]:!text-foreground",
  "[&>h6]:!text-sm [&>h6]:!font-semibold [&>h6]:!leading-6 [&>h6]:!tracking-normal [&>h6]:!text-foreground",
].join(" ");

export function PersonaCatalogDialog({
  createContent,
  error,
  feedbackErrorMessage,
  feedbackNoticeMessage,
  isLoading,
  isPending,
  onClearFeedback,
  onImportFile,
  onOpenChange,
  onSelectPersona,
  open,
  personas,
}: PersonaCatalogDialogProps) {
  const contentRef = React.useRef<HTMLDivElement | null>(null);
  const fileInputRef = React.useRef<HTMLInputElement | null>(null);
  const dragDepthRef = React.useRef(0);
  const createDirtyRef = React.useRef(false);
  const [isDragOver, setIsDragOver] = React.useState(false);
  const [pendingNavigation, setPendingNavigation] =
    React.useState<PendingNavigation | null>(null);
  const [selection, setSelection] = React.useState("create");
  const selectedPersonaId = selection.startsWith("persona:")
    ? selection.slice("persona:".length)
    : null;
  const selectedPersona = React.useMemo(() => {
    if (!selectedPersonaId) {
      return null;
    }

    return personas.find((persona) => persona.id === selectedPersonaId) ?? null;
  }, [personas, selectedPersonaId]);

  React.useEffect(() => {
    if (open) {
      createDirtyRef.current = false;
      setSelection("create");
      setPendingNavigation(null);
      dragDepthRef.current = 0;
      setIsDragOver(false);
    }
  }, [open]);

  React.useEffect(() => {
    if (
      selectedPersonaId &&
      !personas.some((persona) => persona.id === selectedPersonaId)
    ) {
      setSelection("create");
    }
  }, [personas, selectedPersonaId]);

  useFeedbackToasts(feedbackNoticeMessage, feedbackErrorMessage);

  const selectedPersonaIsActive = selectedPersona
    ? isCatalogPersonaSelected(selectedPersona)
    : false;

  const handleUseSelectedPersona = () => {
    if (!selectedPersona || selectedPersonaIsActive) {
      return;
    }

    onClearFeedback();
    onSelectPersona(selectedPersona, true);
  };

  const isImportSelected = selection === "import";
  const handleCreateDirtyChange = React.useCallback((dirty: boolean) => {
    createDirtyRef.current = dirty;
  }, []);

  function requestSelection(nextSelection: string) {
    if (
      selection === "create" &&
      nextSelection !== "create" &&
      createDirtyRef.current
    ) {
      setPendingNavigation({
        type: "selection",
        selection: nextSelection,
      });
      return;
    }
    setSelection(nextSelection);
  }

  function requestClose() {
    if (selection === "create" && createDirtyRef.current) {
      setPendingNavigation({ type: "close" });
      return;
    }
    onOpenChange(false);
  }

  function discardChangesAndNavigate() {
    const navigation = pendingNavigation;
    createDirtyRef.current = false;
    setPendingNavigation(null);
    if (navigation?.type === "selection") {
      setSelection(navigation.selection);
    } else if (navigation?.type === "close") {
      onOpenChange(false);
    }
  }

  React.useEffect(() => {
    if (!isImportSelected) {
      dragDepthRef.current = 0;
      setIsDragOver(false);
    }
  }, [isImportSelected]);

  function hasFiles(event: React.DragEvent) {
    return event.dataTransfer.types.includes("Files");
  }

  function isAgentSnapshot(file: File) {
    const lowerName = file.name.toLowerCase();
    return (
      lowerName.endsWith(".agent.json") || lowerName.endsWith(".agent.png")
    );
  }

  async function importFile(file: File) {
    if (!isAgentSnapshot(file)) return;
    const buffer = await file.arrayBuffer();
    onOpenChange(false);
    onImportFile(Array.from(new Uint8Array(buffer)), file.name);
  }

  return (
    <>
      <Dialog
        onOpenChange={(nextOpen) => {
          if (!nextOpen && isPending) return;
          if (!nextOpen) {
            requestClose();
            return;
          }
          onOpenChange(true);
        }}
        open={open}
      >
        <ChooserDialogContent
          className="h-[42rem] max-w-5xl"
          contentClassName="flex min-h-0 min-w-0 flex-1 p-0"
          data-testid="persona-catalog-dialog"
          description={personaCatalogCopy.dialogDescription}
          headerClassName="bg-sidebar pb-3 text-sidebar-foreground"
          headerTestId="persona-catalog-dialog-header"
          onOpenAutoFocus={(event) => {
            event.preventDefault();
            contentRef.current?.focus();
          }}
          ref={contentRef}
          scrollAreaClassName="flex min-h-0 overflow-hidden px-0"
          scrollAreaTestId="persona-catalog-dialog-body"
          tabIndex={-1}
          title={personaCatalogCopy.dialogTitle}
          onDragEnter={(event) => {
            if (!isImportSelected || !hasFiles(event)) return;
            event.preventDefault();
            dragDepthRef.current += 1;
            setIsDragOver(true);
          }}
          onDragLeave={(event) => {
            if (!isImportSelected) return;
            event.preventDefault();
            dragDepthRef.current = Math.max(0, dragDepthRef.current - 1);
            if (dragDepthRef.current === 0) setIsDragOver(false);
          }}
          onDragOver={(event) => {
            if (!isImportSelected || !hasFiles(event)) return;
            event.preventDefault();
            event.dataTransfer.dropEffect = "copy";
          }}
          onDrop={(event) => {
            if (!isImportSelected || !hasFiles(event)) return;
            event.preventDefault();
            dragDepthRef.current = 0;
            setIsDragOver(false);
            const file = event.dataTransfer.files[0];
            if (file) void importFile(file);
          }}
        >
          <PersonaCatalogChooser
            createContent={createContent({
              onDirtyChange: handleCreateDirtyChange,
              onRequestClose: requestClose,
            })}
            error={error}
            isDragOver={isDragOver}
            isLoading={isLoading}
            isPending={isPending}
            onImport={() => fileInputRef.current?.click()}
            isSelectedPersonaActive={selectedPersonaIsActive}
            onUsePersona={handleUseSelectedPersona}
            onSelectionChange={requestSelection}
            personas={personas}
            selection={selection}
            selectedPersona={selectedPersona}
            selectedPersonaId={selectedPersona?.id ?? null}
          />
          <input
            accept=".agent.json,.agent.png"
            className="hidden"
            data-testid="agent-catalog-import-input"
            onChange={(event) => {
              const file = event.target.files?.[0];
              if (file) void importFile(file);
              event.target.value = "";
            }}
            ref={fileInputRef}
            type="file"
          />
        </ChooserDialogContent>
      </Dialog>

      <AlertDialog
        onOpenChange={(nextOpen) => {
          if (!nextOpen) setPendingNavigation(null);
        }}
        open={pendingNavigation !== null}
      >
        <AlertDialogContent data-testid="discard-create-agent-dialog">
          <AlertDialogHeader>
            <AlertDialogTitle>Discard agent changes?</AlertDialogTitle>
            <AlertDialogDescription>
              Your changes to this agent will be lost.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep editing</AlertDialogCancel>
            <AlertDialogAction asChild>
              <Button onClick={discardChangesAndNavigate} variant="destructive">
                Discard changes
              </Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

type PersonaCatalogChooserProps = {
  createContent: React.ReactNode;
  error: Error | null;
  isDragOver: boolean;
  isLoading: boolean;
  isPending: boolean;
  isSelectedPersonaActive: boolean;
  onImport: () => void;
  onUsePersona: () => void;
  onSelectionChange: (selection: string) => void;
  personas: AgentPersona[];
  selection: string;
  selectedPersona: AgentPersona | null;
  selectedPersonaId: string | null;
};

function PersonaCatalogChooser({
  createContent,
  error,
  isDragOver,
  isLoading,
  isPending,
  isSelectedPersonaActive,
  onImport,
  onUsePersona,
  onSelectionChange,
  personas,
  selection,
  selectedPersona,
  selectedPersonaId,
}: PersonaCatalogChooserProps) {
  return (
    <div className="relative flex min-h-0 flex-1 flex-col overflow-hidden bg-sidebar sm:flex-row">
      {selection === "import" && isDragOver ? (
        <div
          className="pointer-events-none absolute inset-2 z-50 flex items-center justify-center rounded-2xl border-2 border-dashed border-primary bg-primary/10 backdrop-blur-sm"
          data-testid="agent-catalog-drop-overlay"
        >
          <p className="rounded-full bg-background/90 px-4 py-2 text-sm font-medium text-primary shadow-sm">
            Drop .agent.json or .agent.png to import
          </p>
        </div>
      ) : null}
      <div className="flex max-h-56 min-h-0 flex-col sm:max-h-none sm:w-56">
        <div
          className="min-h-0 flex-1 overflow-y-auto px-2 py-3"
          data-testid="persona-catalog-dialog-scroll-area"
        >
          <div className="space-y-1">
            <CatalogNavigationButton
              icon={<Plus className="h-4 w-4" />}
              isCurrent={selection === "create"}
              label="Create agent"
              onClick={() => onSelectionChange("create")}
              testId="agent-catalog-create"
            />
            <CatalogNavigationButton
              icon={<Upload className="h-4 w-4" />}
              isCurrent={selection === "import"}
              label="Import"
              onClick={() => onSelectionChange("import")}
              testId="agent-catalog-import"
            />
          </div>

          <div className="my-3 border-t border-sidebar-border/60" />

          {isLoading ? <PersonaCatalogListSkeleton /> : null}

          {!isLoading && personas.length > 0 ? (
            <div className="space-y-1">
              {personas.map((persona) => {
                const isCurrent = persona.id === selectedPersonaId;

                return (
                  <button
                    aria-current={isCurrent ? "true" : undefined}
                    className={cn(
                      "flex w-full items-center gap-2 rounded-lg px-4 py-1.5 text-left transition-[background-color,color,box-shadow] focus:outline-hidden focus-visible:ring-2 focus-visible:ring-sidebar-ring/50 focus-visible:ring-offset-2 focus-visible:ring-offset-sidebar",
                      isCurrent
                        ? "bg-sidebar-active text-sidebar-active-foreground"
                        : "text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
                    )}
                    data-testid={`persona-catalog-list-item-${persona.id}`}
                    key={persona.id}
                    onClick={() => {
                      onSelectionChange(`persona:${persona.id}`);
                    }}
                    type="button"
                  >
                    <ProfileAvatar
                      avatarUrl={persona.avatarUrl}
                      className="h-6 w-6 text-3xs"
                      label={persona.displayName}
                    />
                    <span className="min-w-0 flex-1 truncate text-sm font-medium">
                      {persona.displayName}
                    </span>
                  </button>
                );
              })}
            </div>
          ) : null}
          {!isLoading && personas.length === 0 && !error ? (
            <p className="px-4 py-2 text-xs text-sidebar-foreground/50">
              No shared agents
            </p>
          ) : null}
        </div>
      </div>

      <div className="relative z-10 mb-3 ml-px mr-3 flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-xl bg-background shadow-[-1px_0_0_0_hsl(var(--sidebar-border)/0.45)]">
        {selection === "create" ? createContent : null}
        {selection === "import" ? (
          <ImportAgentPane onImport={onImport} />
        ) : null}
        {selectedPersona ? (
          <>
            <div
              className="min-h-0 min-w-0 max-w-full flex-1 overflow-x-hidden overflow-y-auto px-5 pb-20 pt-5"
              data-testid="persona-catalog-detail-pane"
            >
              <PersonaCatalogDetail persona={selectedPersona} />
            </div>
            <div className="pointer-events-none absolute inset-x-0 bottom-0 flex justify-end bg-linear-to-t from-background via-background/95 to-transparent px-4 pb-3 pt-10">
              <Button
                aria-label={
                  isSelectedPersonaActive
                    ? `${selectedPersona.displayName} is already in My Agents`
                    : `Add ${selectedPersona.displayName} from Agent Catalog`
                }
                className="pointer-events-auto"
                data-testid={`persona-catalog-use-agent-target-${selectedPersona.id}`}
                disabled={isSelectedPersonaActive || isPending}
                onClick={onUsePersona}
                type="button"
              >
                {isSelectedPersonaActive
                  ? personaCatalogCopy.addedAction
                  : personaCatalogCopy.useAction}
              </Button>
            </div>
          </>
        ) : null}
        {selection.startsWith("persona:") && isLoading ? (
          <div className="min-h-0 flex-1 overflow-y-auto px-5 py-5">
            <PersonaCatalogDetailSkeleton />
          </div>
        ) : null}
        {error ? (
          <p className="m-5 rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            {error.message}
          </p>
        ) : null}
      </div>
    </div>
  );
}

function CatalogNavigationButton({
  icon,
  isCurrent,
  label,
  onClick,
  testId,
}: {
  icon: React.ReactNode;
  isCurrent: boolean;
  label: string;
  onClick: () => void;
  testId: string;
}) {
  return (
    <button
      aria-current={isCurrent ? "true" : undefined}
      className={cn(
        "flex w-full items-center gap-2 rounded-lg px-4 py-2 text-left text-sm font-medium transition-[background-color,color,box-shadow] focus:outline-hidden focus-visible:ring-2 focus-visible:ring-sidebar-ring/50 focus-visible:ring-offset-2 focus-visible:ring-offset-sidebar",
        isCurrent
          ? "bg-sidebar-active text-sidebar-active-foreground"
          : "text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
      )}
      data-testid={testId}
      onClick={onClick}
      type="button"
    >
      {icon}
      <span>{label}</span>
    </button>
  );
}

function ImportAgentPane({ onImport }: { onImport: () => void }) {
  return (
    <button
      className="m-5 flex min-h-0 flex-1 flex-col items-center justify-center rounded-xl border-2 border-dashed border-border bg-muted/20 px-8 py-12 text-center transition-colors hover:border-primary/60 hover:bg-primary/5 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
      data-testid="agent-catalog-import-dropzone"
      onClick={onImport}
      type="button"
    >
      <span className="flex h-14 w-14 items-center justify-center rounded-full bg-muted text-muted-foreground">
        <Upload className="h-6 w-6" />
      </span>
      <span className="mt-4 text-base font-semibold">Import an agent</span>
      <span className="mt-2 max-w-sm text-sm text-muted-foreground">
        Drop an .agent.json or .agent.png file anywhere in this window, or
        choose a file.
      </span>
      <span className="mt-5 rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground">
        Choose file
      </span>
    </button>
  );
}

/**
 * Derives the "Added by" label for a catalog entry from a resolved profile
 * summary. Prefers `displayName`, falls back to `name`, then to the default
 * "Community member" string when both are absent, null, or whitespace-only.
 */
export function resolveCatalogOwnerLabel(
  summary:
    | { displayName?: string | null; name?: string | null }
    | null
    | undefined,
): string {
  return (
    summary?.displayName?.trim() || summary?.name?.trim() || "Community member"
  );
}

function PersonaCatalogDetail({ persona }: { persona: AgentPersona }) {
  const isCommunityEntry =
    isCatalogPersona(persona) && !persona.catalogSource.isOwn;
  const ownerPubkey = isCommunityEntry
    ? persona.catalogSource.ownerPubkey
    : undefined;
  const ownerBatchQuery = useUsersBatchQuery(ownerPubkey ? [ownerPubkey] : [], {
    enabled: !!ownerPubkey,
  });

  let addedByLabel: string;
  if (!isCommunityEntry) {
    addedByLabel = "You";
  } else {
    const summary = ownerPubkey
      ? ownerBatchQuery.data?.profiles[ownerPubkey.toLowerCase()]
      : undefined;
    addedByLabel = resolveCatalogOwnerLabel(summary);
  }

  return (
    <div className="w-full min-w-0 max-w-full space-y-6 overflow-x-hidden">
      <div className="flex items-center gap-3">
        <ProfileAvatar
          avatarUrl={persona.avatarUrl}
          className="h-12 w-12 text-sm"
          label={persona.displayName}
        />
        <div className="min-w-0">
          <h3 className="truncate text-xl font-semibold leading-snug">
            {persona.displayName}
          </h3>
          {persona.isBuiltIn ? null : (
            <PersonaAddedBy className="mt-0.5" label={addedByLabel} />
          )}
        </div>
      </div>

      <AgentDefinitionMetadata
        isBuiltIn={persona.isBuiltIn}
        model={persona.model}
        runtime={persona.runtime}
      />

      <div className="min-w-0 max-w-full pt-3">
        <p className="text-base font-semibold text-foreground">
          Agent instruction
        </p>
        <Markdown
          className={agentInstructionMarkdownClassName}
          content={persona.systemPrompt}
          interactive={false}
        />
      </div>
    </div>
  );
}

function PersonaCatalogListSkeleton() {
  return (
    <div className="space-y-2">
      {["first", "second", "third", "fourth", "fifth"].map((key) => (
        <div
          className="flex items-center gap-2 rounded-lg px-4 py-1.5"
          key={key}
        >
          <Skeleton className="h-6 w-6 rounded-full" />
          <Skeleton className="h-4 w-28" />
        </div>
      ))}
    </div>
  );
}

function PersonaCatalogDetailSkeleton() {
  return (
    <div className="space-y-6">
      <div className="flex items-center gap-3">
        <Skeleton className="h-12 w-12 rounded-full" />
        <Skeleton className="h-6 w-40" />
      </div>
      <div className="grid overflow-hidden rounded-lg border border-border/70 sm:grid-cols-3">
        <Skeleton className="h-20 rounded-none" />
        <Skeleton className="h-20 rounded-none" />
        <Skeleton className="h-20 rounded-none" />
      </div>
      <Skeleton className="h-48 rounded-lg" />
    </div>
  );
}
