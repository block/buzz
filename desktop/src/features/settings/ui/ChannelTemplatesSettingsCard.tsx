import {
  ArrowLeft,
  Copy,
  Folder,
  FolderPlus,
  GitBranch,
  MessageSquare,
  MoreHorizontal,
  Pencil,
  Plus,
  Trash2,
  X,
} from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import * as React from "react";
import { toast } from "sonner";

import { usePersonasQuery, useTeamsQuery } from "@/features/agents/hooks";
import {
  useChannelTemplatesQuery,
  useCreateChannelTemplateMutation,
  useDeleteChannelTemplateMutation,
  useDuplicateChannelTemplateMutation,
  useUpdateChannelTemplateMutation,
} from "@/features/channel-templates/hooks";
import { pickChannelTemplateProjectFolder } from "@/shared/api/tauriChannelTemplates";
import {
  CHANNEL_FORM_FIELD_CONTROL_CLASS,
  CHANNEL_FORM_FIELD_SHELL_CLASS,
} from "@/features/channels/ui/channelFormStyles";
import type {
  ChannelTemplate,
  CreateChannelTemplateInput,
  UpdateChannelTemplateInput,
} from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { SettingsSectionHeader } from "./SettingsSectionHeader";
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
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Input } from "@/shared/ui/input";
import {
  TemplateAgentPicker,
  TemplateAgentSummary,
} from "./ChannelTemplateAgentPicker";
import {
  ChannelTemplateCanvasSection,
  ChannelTemplateWorktreeFields,
  DEFAULT_WORKTREE_BASE_BRANCH,
  DEFAULT_WORKTREE_LOCATION,
} from "./ChannelTemplateWorkspaceFields";
import { TemplateStepIndicator } from "./TemplateStepIndicator";
import { ChannelTemplateSavedIdentities } from "./ChannelTemplateSavedIdentities";

const TEMPLATE_FORM_ID = "template-form";
type TemplateFormPage = "details" | "agents";

export function ChannelTemplatesSettingsCard() {
  const templatesQuery = useChannelTemplatesQuery();
  const deleteMutation = useDeleteChannelTemplateMutation();
  const duplicateMutation = useDuplicateChannelTemplateMutation();

  const [editingTemplate, setEditingTemplate] =
    React.useState<ChannelTemplate | null>(null);
  const [isCreateOpen, setIsCreateOpen] = React.useState(false);
  const [deleteTarget, setDeleteTarget] =
    React.useState<ChannelTemplate | null>(null);

  const templates = templatesQuery.data ?? [];

  function handleDuplicate(template: ChannelTemplate) {
    duplicateMutation.mutate(template.id, {
      onSuccess: (created) => {
        toast.success(`Duplicated as "${created.name}"`);
      },
      onError: (error) => {
        toast.error(
          error instanceof Error ? error.message : "Failed to duplicate",
        );
      },
    });
  }

  function handleDelete() {
    if (!deleteTarget) return;
    deleteMutation.mutate(deleteTarget.id, {
      onSuccess: () => {
        toast.success(`Deleted "${deleteTarget.name}"`);
        setDeleteTarget(null);
      },
      onError: (error) => {
        toast.error(
          error instanceof Error ? error.message : "Failed to delete",
        );
      },
    });
  }

  return (
    <section className="min-w-0" data-testid="settings-channel-templates">
      <SettingsSectionHeader
        title="Templates"
        description={<>Save reusable configurations for new channels.</>}
        action={
          <Button
            onClick={() => setIsCreateOpen(true)}
            size="sm"
            type="button"
            variant="outline"
          >
            <Plus className="mr-1.5 h-4 w-4" />
            New template
          </Button>
        }
      />

      {templatesQuery.isLoading ? (
        <p className="py-6 text-center text-sm text-muted-foreground">
          Loading templates...
        </p>
      ) : (
        <TemplateList
          emptyCopy="No templates yet."
          onDelete={setDeleteTarget}
          onDuplicate={handleDuplicate}
          onEdit={setEditingTemplate}
          templates={templates}
        />
      )}

      <TemplateFormDialog
        onOpenChange={setIsCreateOpen}
        open={isCreateOpen}
        template={null}
      />

      {editingTemplate ? (
        <TemplateFormDialog
          onOpenChange={(open) => {
            if (!open) setEditingTemplate(null);
          }}
          open
          template={editingTemplate}
        />
      ) : null}

      <AlertDialog
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null);
        }}
        open={deleteTarget !== null}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete template</AlertDialogTitle>
            <AlertDialogDescription>
              Are you sure you want to delete &quot;{deleteTarget?.name}&quot;?
              This action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={handleDelete}
            >
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}

function TemplateList({
  templates,
  emptyCopy,
  onEdit,
  onDuplicate,
  onDelete,
}: {
  templates: ChannelTemplate[];
  emptyCopy: string;
  onEdit: (template: ChannelTemplate) => void;
  onDuplicate: (template: ChannelTemplate) => void;
  onDelete: (template: ChannelTemplate) => void;
}) {
  return (
    <div className="space-y-3" data-testid="templates-list">
      {templates.length === 0 ? (
        <div className="rounded-xl border border-dashed border-border/70 bg-muted/15 px-4 py-6 text-center text-sm text-muted-foreground">
          {emptyCopy}
        </div>
      ) : (
        templates.map((template) => (
          <TemplateRow
            key={template.id}
            onDelete={() => onDelete(template)}
            onDuplicate={() => onDuplicate(template)}
            onEdit={() => onEdit(template)}
            template={template}
          />
        ))
      )}
    </div>
  );
}

function TemplateRow({
  template,
  onEdit,
  onDuplicate,
  onDelete,
}: {
  template: ChannelTemplate;
  onEdit: () => void;
  onDuplicate: () => void;
  onDelete: () => void;
}) {
  const hasDetails =
    template.agents.personas.length > 0 ||
    template.agents.teams.length > 0 ||
    template.projectFolders.length > 0 ||
    Boolean(template.worktree) ||
    Boolean(template.canvasTemplate);

  return (
    <div
      className="min-h-16 rounded-2xl border border-border/60 bg-muted/20 px-4 py-3.5 text-sm transition-colors hover:bg-muted/30"
      data-testid={`channel-template-row-${template.id}`}
    >
      <div className="flex items-start justify-between gap-4">
        <div className="flex min-w-0 items-center gap-2 pt-1">
          <p className="truncate text-sm font-medium">{template.name}</p>
          {template.isBuiltin ? (
            <Badge className="shrink-0 text-2xs uppercase" variant="outline">
              built-in
            </Badge>
          ) : null}
        </div>

        <DropdownMenu modal={false}>
          <DropdownMenuTrigger asChild>
            <Button
              aria-label={`Open actions for ${template.name}`}
              className="h-7 w-7 shrink-0"
              data-testid={`channel-template-menu-${template.id}`}
              size="icon"
              type="button"
              variant="ghost"
            >
              <MoreHorizontal className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onClick={onEdit}>
              <Pencil className="mr-2 h-4 w-4" />
              Edit
            </DropdownMenuItem>
            <DropdownMenuItem onClick={onDuplicate}>
              <Copy className="mr-2 h-4 w-4" />
              Duplicate
            </DropdownMenuItem>
            {!template.isBuiltin ? (
              <DropdownMenuItem
                className="text-destructive focus:text-destructive"
                onClick={onDelete}
              >
                <Trash2 className="mr-2 h-4 w-4" />
                Delete
              </DropdownMenuItem>
            ) : null}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      {hasDetails ? (
        <dl
          className="mt-3 grid gap-2 sm:grid-cols-2"
          data-testid={`channel-template-details-${template.id}`}
        >
          <ChannelTemplateSavedIdentities template={template} />
          {template.projectFolders.map((projectFolder, index) => (
            <TemplateDetail
              icon={<Folder aria-hidden className="h-4 w-4" />}
              key={projectFolder}
              label={
                template.projectFolders.length > 1
                  ? `Project folder ${index + 1}`
                  : "Project folder"
              }
              testId={
                template.projectFolders.length > 1
                  ? `channel-template-detail-folder-${template.id}-${index}`
                  : `channel-template-detail-folder-${template.id}`
              }
              value={projectFolder}
            />
          ))}
          {template.worktree ? (
            <TemplateDetail
              icon={<GitBranch aria-hidden className="h-4 w-4" />}
              label="Worktree"
              testId={`channel-template-detail-worktree-${template.id}`}
              value={`${template.worktree.location} · ${template.worktree.baseBranch}`}
            />
          ) : null}
          {template.canvasTemplate ? (
            <TemplateDetail
              icon={<MessageSquare aria-hidden className="h-4 w-4" />}
              label="Canvas"
              testId={`channel-template-detail-canvas-${template.id}`}
              value="Included"
            />
          ) : null}
        </dl>
      ) : null}
    </div>
  );
}

function TemplateDetail({
  icon,
  label,
  testId,
  value,
}: {
  icon: React.ReactNode;
  label: string;
  testId: string;
  value: string;
}) {
  return (
    <div
      className="flex min-w-0 items-center gap-3 rounded-lg border border-border/60 bg-background/60 px-3 py-2"
      data-testid={testId}
    >
      <span className="shrink-0 text-muted-foreground">{icon}</span>
      <div className="min-w-0">
        <dt className="text-xs text-muted-foreground">{label}</dt>
        <dd className="truncate text-sm font-medium" title={value}>
          {value}
        </dd>
      </div>
    </div>
  );
}

export function TemplateFormDialog({
  template,
  open,
  onOpenChange,
  onCreated,
}: {
  template: ChannelTemplate | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated?: (template: ChannelTemplate) => void;
}) {
  const isEditing = template !== null;
  const createMutation = useCreateChannelTemplateMutation();
  const updateMutation = useUpdateChannelTemplateMutation();
  const personasQuery = usePersonasQuery();
  const teamsQuery = useTeamsQuery();
  const shouldReduceMotion = useReducedMotion() ?? false;

  const [page, setPage] = React.useState<TemplateFormPage>("details");
  const [pageDirection, setPageDirection] = React.useState<1 | -1>(1);
  const [name, setName] = React.useState("");
  const [canvasTemplate, setCanvasTemplate] = React.useState("");
  const [isCanvasOpen, setIsCanvasOpen] = React.useState(false);
  const [projectFolders, setProjectFolders] = React.useState<string[]>([]);
  const [worktreeEnabled, setWorktreeEnabled] = React.useState(false);
  const [worktreeLocation, setWorktreeLocation] = React.useState("");
  const [worktreeBaseBranch, setWorktreeBaseBranch] = React.useState(
    DEFAULT_WORKTREE_BASE_BRANCH,
  );
  const [isPickingProjectFolder, setIsPickingProjectFolder] =
    React.useState(false);
  const [selectedPersonaIds, setSelectedPersonaIds] = React.useState<string[]>(
    [],
  );
  const [selectedTeamIds, setSelectedTeamIds] = React.useState<string[]>([]);
  const [personaRuntimes, setPersonaRuntimes] = React.useState<
    Record<string, string>
  >({});
  const [teamRuntimes, setTeamRuntimes] = React.useState<
    Record<string, string>
  >({});

  const isPending = createMutation.isPending || updateMutation.isPending;
  const isAgentPage = page === "agents";
  const detailsValid =
    name.trim().length > 0 &&
    (!worktreeEnabled ||
      (projectFolders.length > 0 && worktreeBaseBranch.trim().length > 0));

  React.useEffect(() => {
    if (!open) return;
    setPage("details");
    setPageDirection(1);
    setIsCanvasOpen(false);
    if (template) {
      setName(template.name);
      setCanvasTemplate(template.canvasTemplate ?? "");
      setProjectFolders(template.projectFolders);
      setWorktreeEnabled(template.worktree !== null);
      setWorktreeLocation(template.worktree?.location ?? "");
      setWorktreeBaseBranch(
        template.worktree?.baseBranch ?? DEFAULT_WORKTREE_BASE_BRANCH,
      );
      setSelectedPersonaIds(template.agents.personas.map((p) => p.personaId));
      setSelectedTeamIds(template.agents.teams.map((t) => t.teamId));
      const pRuntimes: Record<string, string> = {};
      for (const p of template.agents.personas) {
        if (p.runtime) pRuntimes[p.personaId] = p.runtime;
      }
      setPersonaRuntimes(pRuntimes);
      const tRuntimes: Record<string, string> = {};
      for (const t of template.agents.teams) {
        if (t.runtime) tRuntimes[t.teamId] = t.runtime;
      }
      setTeamRuntimes(tRuntimes);
    } else {
      setName("");
      setCanvasTemplate("");
      setProjectFolders([]);
      setWorktreeEnabled(false);
      setWorktreeLocation("");
      setWorktreeBaseBranch(DEFAULT_WORKTREE_BASE_BRANCH);
      setSelectedPersonaIds([]);
      setSelectedTeamIds([]);
      setPersonaRuntimes({});
      setTeamRuntimes({});
    }
  }, [open, template]);

  function showAgentPage() {
    setPageDirection(1);
    setPage("agents");
  }

  function showDetailsPage() {
    setPageDirection(-1);
    setPage("details");
  }

  function handleSubmit(event: React.FormEvent) {
    event.preventDefault();
    if (!isEditing && page === "details") {
      if (detailsValid) showAgentPage();
      return;
    }
    if (isEditing && page === "agents") {
      showDetailsPage();
      return;
    }
    const trimmedName = name.trim();
    if (!trimmedName) return;

    const agents = {
      personas: selectedPersonaIds.map((personaId) => ({
        personaId,
        runtime: personaRuntimes[personaId] || null,
        model: null,
        role: null,
        backend: null,
      })),
      teams: selectedTeamIds.map((teamId) => ({
        teamId,
        runtime: teamRuntimes[teamId] || null,
        model: null,
        backend: null,
      })),
    };

    if (isEditing) {
      const input: UpdateChannelTemplateInput = {
        id: template.id,
        name: trimmedName,
        // The current editor intentionally does not expose channel type or
        // visibility, so preserve the stored values when updating unrelated
        // template fields rather than letting the backend's defaults overwrite
        // them.
        channelType: template.channelType,
        visibility: template.visibility,
        description: template.description ?? undefined,
        canvasTemplate: canvasTemplate.trim() || undefined,
        projectFolders,
        worktree: worktreeEnabled
          ? {
              location: worktreeLocation.trim() || DEFAULT_WORKTREE_LOCATION,
              baseBranch: worktreeBaseBranch.trim(),
            }
          : undefined,
        agents,
      };

      updateMutation.mutate(input, {
        onSuccess: () => {
          toast.success(`Updated "${trimmedName}"`);
          onOpenChange(false);
        },
        onError: (error) => {
          toast.error(
            error instanceof Error ? error.message : "Failed to update",
          );
        },
      });
    } else {
      const input: CreateChannelTemplateInput = {
        name: trimmedName,
        canvasTemplate: canvasTemplate.trim() || undefined,
        projectFolders,
        worktree: worktreeEnabled
          ? {
              location: worktreeLocation.trim() || DEFAULT_WORKTREE_LOCATION,
              baseBranch: worktreeBaseBranch.trim(),
            }
          : undefined,
        agents,
      };

      createMutation.mutate(input, {
        onSuccess: (created) => {
          toast.success(`Created "${trimmedName}"`);
          onCreated?.(created);
          onOpenChange(false);
        },
        onError: (error) => {
          toast.error(
            error instanceof Error ? error.message : "Failed to create",
          );
        },
      });
    }
  }

  async function handlePickProjectFolder() {
    setIsPickingProjectFolder(true);
    try {
      const selectedFolders = await pickChannelTemplateProjectFolder();
      if (selectedFolders.length > 0) {
        setProjectFolders((current) =>
          Array.from(new Set([...current, ...selectedFolders])),
        );
      }
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Failed to choose a folder",
      );
    } finally {
      setIsPickingProjectFolder(false);
    }
  }

  function handleTogglePersona(personaId: string) {
    setSelectedPersonaIds((prev) => {
      if (prev.includes(personaId)) {
        setPersonaRuntimes((pp) => {
          const next = { ...pp };
          delete next[personaId];
          return next;
        });
        return prev.filter((id) => id !== personaId);
      }
      return [...prev, personaId];
    });
  }

  function handleToggleTeam(teamId: string) {
    setSelectedTeamIds((prev) => {
      if (prev.includes(teamId)) {
        setTeamRuntimes((tp) => {
          const next = { ...tp };
          delete next[teamId];
          return next;
        });
        return prev.filter((id) => id !== teamId);
      }
      return [...prev, teamId];
    });
  }

  const dialogHeight =
    isEditing || isAgentPage || isCanvasOpen || worktreeEnabled
      ? "min(42rem, 85vh)"
      : "min(31rem, 85vh)";

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <ChooserDialogContent
        className="max-w-xl"
        contentClassName="h-full pt-3"
        data-testid="channel-template-dialog"
        title={isEditing ? "Edit template" : "Create new template"}
        headerSubtitle={
          isEditing
            ? isAgentPage
              ? "Choose the agent teams and agents included in this template."
              : "Update template details and review its included agents."
            : undefined
        }
        footer={
          <div className="flex w-full items-center justify-between gap-3">
            {isEditing && isAgentPage ? (
              <Button
                disabled={isPending}
                onClick={showDetailsPage}
                type="button"
                variant="ghost"
              >
                <ArrowLeft className="mr-1.5 h-4 w-4" />
                Back
              </Button>
            ) : !isEditing && isAgentPage ? (
              <Button
                disabled={isPending}
                onClick={showDetailsPage}
                type="button"
                variant="ghost"
              >
                <ArrowLeft className="mr-1.5 h-4 w-4" />
                Back
              </Button>
            ) : (
              <span />
            )}
            <div className="flex items-center justify-end gap-2">
              {isEditing && !isAgentPage ? (
                <Button
                  disabled={isPending || isPickingProjectFolder}
                  onClick={() => onOpenChange(false)}
                  type="button"
                  variant="outline"
                >
                  Cancel
                </Button>
              ) : null}
              {isEditing && isAgentPage ? (
                <Button
                  disabled={isPending}
                  key="finish-agent-selection"
                  onClick={showDetailsPage}
                  type="button"
                >
                  Done
                </Button>
              ) : !isEditing && !isAgentPage ? (
                <Button
                  disabled={
                    isPending || isPickingProjectFolder || !detailsValid
                  }
                  key="show-agent-selection"
                  onClick={showAgentPage}
                  type="button"
                >
                  Next
                </Button>
              ) : (
                <Button
                  disabled={
                    isPending || isPickingProjectFolder || !detailsValid
                  }
                  form={TEMPLATE_FORM_ID}
                  key="submit-template"
                  type="submit"
                >
                  {isPending
                    ? isEditing
                      ? "Saving..."
                      : "Creating..."
                    : isEditing
                      ? "Save"
                      : "Create"}
                </Button>
              )}
            </div>
          </div>
        }
        footerClassName="border-t-0 pt-0"
        headerClassName="pb-2"
        scrollAreaClassName="overflow-hidden"
        scrollAreaTestId="channel-template-scroll-area"
        style={{
          height: dialogHeight,
          maxHeight: "min(42rem, 85vh)",
          transitionDuration: shouldReduceMotion ? "0ms" : "220ms",
          transitionProperty: "height, max-width",
          transitionTimingFunction: "cubic-bezier(0.77, 0, 0.175, 1)",
        }}
      >
        <form
          className="flex h-full min-h-0 flex-col"
          id={TEMPLATE_FORM_ID}
          onSubmit={handleSubmit}
        >
          {!isEditing ? (
            <div className="mb-5 shrink-0">
              <TemplateStepIndicator step={isAgentPage ? 2 : 1} />
            </div>
          ) : null}
          <AnimatePresence initial={false} mode="popLayout">
            <motion.div
              animate={{ opacity: 1, transform: "translateX(0px)" }}
              className="flex h-full min-h-0 flex-1 flex-col"
              exit={
                shouldReduceMotion
                  ? { opacity: 0 }
                  : {
                      opacity: 0,
                      transform: `translateX(${pageDirection * -8}px)`,
                    }
              }
              initial={
                shouldReduceMotion
                  ? false
                  : {
                      opacity: 0,
                      transform: `translateX(${pageDirection * 12}px)`,
                    }
              }
              key={page}
              transition={{
                duration: shouldReduceMotion ? 0 : 0.18,
                ease: [0.23, 1, 0.32, 1],
              }}
            >
              {!isAgentPage ? (
                <div
                  className="min-h-0 flex-1 overflow-y-auto pr-1"
                  data-testid="channel-template-details-page"
                >
                  <div className="flex min-h-full flex-col gap-5">
                    <div className="space-y-1.5">
                      <label
                        className="text-sm font-medium text-foreground"
                        htmlFor="template-name"
                      >
                        Template name
                      </label>
                      <div
                        className={cn(
                          "flex min-h-11 items-center px-3",
                          CHANNEL_FORM_FIELD_SHELL_CLASS,
                        )}
                      >
                        <Input
                          autoComplete="off"
                          autoCorrect="off"
                          className={cn(
                            "h-8 px-0 py-0 leading-6",
                            CHANNEL_FORM_FIELD_CONTROL_CLASS,
                            "text-foreground",
                          )}
                          disabled={isPending}
                          id="template-name"
                          onChange={(event) => setName(event.target.value)}
                          placeholder="Template name"
                          spellCheck={false}
                          value={name}
                        />
                      </div>
                    </div>

                    <div className="space-y-1.5">
                      <span className="text-sm font-medium text-foreground">
                        Where do you want to work from?
                      </span>
                      <div
                        className={cn(
                          "divide-y divide-border/60 overflow-hidden",
                          CHANNEL_FORM_FIELD_SHELL_CLASS,
                        )}
                        data-testid="channel-template-project-folders"
                      >
                        {projectFolders.map((projectFolder, index) => (
                          <div
                            className="flex min-h-11 items-center gap-3 px-3"
                            data-testid={`channel-template-project-folder-${index}`}
                            key={projectFolder}
                          >
                            <Folder className="h-4 w-4 shrink-0 text-muted-foreground" />
                            <span
                              className="min-w-0 flex-1 truncate text-sm text-foreground"
                              title={projectFolder}
                            >
                              {projectFolder}
                            </span>
                            <Button
                              aria-label={`Remove ${projectFolder}`}
                              className="-mr-2 h-8 w-8 shrink-0 text-muted-foreground"
                              disabled={isPending || isPickingProjectFolder}
                              onClick={() =>
                                setProjectFolders((current) =>
                                  current.filter(
                                    (folder) => folder !== projectFolder,
                                  ),
                                )
                              }
                              size="icon"
                              type="button"
                              variant="ghost"
                            >
                              <X className="h-4 w-4" />
                            </Button>
                          </div>
                        ))}
                        <button
                          className="flex min-h-11 w-full items-center gap-3 px-3 py-2.5 text-left text-sm text-muted-foreground disabled:pointer-events-none disabled:opacity-50"
                          data-testid="channel-template-add-project-folder"
                          disabled={isPending || isPickingProjectFolder}
                          onClick={() => void handlePickProjectFolder()}
                          type="button"
                        >
                          <FolderPlus className="h-4 w-4 shrink-0" />
                          <span>
                            {isPickingProjectFolder
                              ? "Choosing folder..."
                              : "Add folder"}
                          </span>
                        </button>
                      </div>
                    </div>

                    <ChannelTemplateWorktreeFields
                      baseBranch={worktreeBaseBranch}
                      disabled={isPending}
                      enabled={worktreeEnabled}
                      location={worktreeLocation}
                      onBaseBranchChange={setWorktreeBaseBranch}
                      onEnabledChange={setWorktreeEnabled}
                      onLocationChange={setWorktreeLocation}
                      projectFolders={projectFolders}
                    />

                    <ChannelTemplateCanvasSection
                      disabled={isPending}
                      onChange={setCanvasTemplate}
                      onOpenChange={setIsCanvasOpen}
                      open={isCanvasOpen}
                      value={canvasTemplate}
                    />

                    {isEditing ? (
                      <TemplateAgentSummary
                        isLoading={
                          personasQuery.isLoading || teamsQuery.isLoading
                        }
                        isPending={isPending}
                        onEdit={showAgentPage}
                        personas={personasQuery.data ?? []}
                        selectedPersonaIds={selectedPersonaIds}
                        selectedTeamIds={selectedTeamIds}
                        teams={teamsQuery.data ?? []}
                      />
                    ) : null}
                  </div>
                </div>
              ) : (
                <div
                  className="flex min-h-0 flex-1 flex-col"
                  data-testid="channel-template-agents-page"
                >
                  <TemplateAgentPicker
                    isLoading={personasQuery.isLoading || teamsQuery.isLoading}
                    isPending={isPending}
                    onToggleMember={handleTogglePersona}
                    onToggleTeam={handleToggleTeam}
                    personas={personasQuery.data ?? []}
                    selectedPersonaIds={selectedPersonaIds}
                    selectedTeamIds={selectedTeamIds}
                    showTitle={!isEditing}
                    teams={teamsQuery.data ?? []}
                  />
                </div>
              )}
            </motion.div>
          </AnimatePresence>
        </form>
      </ChooserDialogContent>
    </Dialog>
  );
}
