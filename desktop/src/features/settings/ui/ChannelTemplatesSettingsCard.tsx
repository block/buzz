import {
  Copy,
  MessageSquare,
  MoreHorizontal,
  Pencil,
  Plus,
  Trash2,
  Users,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import {
  useAvailableAcpRuntimes,
  usePersonasQuery,
  useTeamsQuery,
} from "@/features/agents/hooks";
import {
  useChannelTemplatesQuery,
  useCreateChannelTemplateMutation,
  useDeleteChannelTemplateMutation,
  useDuplicateChannelTemplateMutation,
  useUpdateChannelTemplateMutation,
} from "@/features/channel-templates/hooks";
import { AddChannelBotPersonasSection } from "@/features/channels/ui/AddChannelBotPersonasSection";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type {
  AcpRuntime,
  AgentPersona,
  AgentTeam,
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
import { Textarea } from "@/shared/ui/textarea";

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
        title="Channel Templates"
        description={
          <>
            Save reusable channel configurations and apply them when creating
            new channels.
          </>
        }
        action={
          <Button
            onClick={() => setIsCreateOpen(true)}
            size="sm"
            type="button"
            variant="outline"
          >
            <Plus className="mr-1.5 h-4 w-4" />
            Create
          </Button>
        }
      />

      {templatesQuery.isLoading ? (
        <p className="py-6 text-center text-sm text-muted-foreground">
          Loading templates...
        </p>
      ) : templates.length === 0 ? (
        <div className="rounded-xl border border-dashed border-border/70 bg-muted/15 px-4 py-8 text-center text-sm text-muted-foreground">
          No templates yet. Create one to save a reusable channel configuration.
        </div>
      ) : (
        <div className="space-y-1">
          {templates.map((template) => (
            <TemplateRow
              key={template.id}
              onDelete={() => setDeleteTarget(template)}
              onDuplicate={() => handleDuplicate(template)}
              onEdit={() => setEditingTemplate(template)}
              template={template}
            />
          ))}
        </div>
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
  const agentCount =
    template.agents.personas.length + template.agents.teams.length;

  return (
    <div className="group flex items-center gap-3 rounded-lg px-3 py-2.5 hover:bg-muted/50">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-medium">{template.name}</span>
          {template.isBuiltin ? (
            <Badge className="shrink-0 text-2xs uppercase" variant="outline">
              built-in
            </Badge>
          ) : null}
        </div>
        {template.description ? (
          <p className="mt-0.5 truncate text-sm font-normal text-muted-foreground">
            {template.description}
          </p>
        ) : null}
        <div className="mt-1 flex items-center gap-3 text-xs text-muted-foreground">
          {agentCount > 0 ? (
            <span className="flex items-center gap-1">
              <Users className="h-4 w-4" />
              {agentCount} {agentCount === 1 ? "agent" : "agents"}
            </span>
          ) : null}
          {template.canvasTemplate ? (
            <span className="flex items-center gap-1">
              <MessageSquare className="h-4 w-4" />
              canvas
            </span>
          ) : null}
        </div>
      </div>

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            className="h-7 w-7 shrink-0 opacity-0 group-hover:opacity-100"
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
  );
}

function TemplateFormDialog({
  template,
  open,
  onOpenChange,
}: {
  template: ChannelTemplate | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const isEditing = template !== null;
  const createMutation = useCreateChannelTemplateMutation();
  const updateMutation = useUpdateChannelTemplateMutation();
  const personasQuery = usePersonasQuery();
  const teamsQuery = useTeamsQuery();
  const providersQuery = useAvailableAcpRuntimes();
  const runtimes = providersQuery.data ?? [];

  const [name, setName] = React.useState("");
  const [description, setDescription] = React.useState("");
  const [canvasTemplate, setCanvasTemplate] = React.useState("");
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

  React.useEffect(() => {
    if (!open) return;
    if (template) {
      setName(template.name);
      setDescription(template.description ?? "");
      setCanvasTemplate(template.canvasTemplate ?? "");
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
      setDescription("");
      setCanvasTemplate("");
      setSelectedPersonaIds([]);
      setSelectedTeamIds([]);
      setPersonaRuntimes({});
      setTeamRuntimes({});
    }
  }, [open, template]);

  function handleSubmit(event: React.FormEvent) {
    event.preventDefault();
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
        description: description.trim() || undefined,
        canvasTemplate: canvasTemplate.trim() || undefined,
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
        description: description.trim() || undefined,
        canvasTemplate: canvasTemplate.trim() || undefined,
        agents,
      };

      createMutation.mutate(input, {
        onSuccess: () => {
          toast.success(`Created "${trimmedName}"`);
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

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <ChooserDialogContent
        className="max-w-lg"
        title={isEditing ? "Edit template" : "Create template"}
        description={
          isEditing
            ? "Update this channel template configuration."
            : "Save a reusable channel configuration."
        }
        footer={
          <div className="flex w-full items-center justify-end gap-2">
            <Button
              disabled={isPending}
              onClick={() => onOpenChange(false)}
              type="button"
              variant="ghost"
            >
              Cancel
            </Button>
            <Button
              disabled={isPending || name.trim().length === 0}
              form="template-form"
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
          </div>
        }
      >
        <form className="space-y-5" id="template-form" onSubmit={handleSubmit}>
          {/* Name */}
          <div className="space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="template-name"
            >
              Name
            </label>
            <Input
              autoComplete="off"
              disabled={isPending}
              id="template-name"
              onChange={(e) => setName(e.target.value)}
              placeholder="Sprint Planning"
              value={name}
            />
          </div>

          {/* Description */}
          <div className="space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="template-description"
            >
              Description{" "}
              <span className="font-normal text-muted-foreground">
                (optional)
              </span>
            </label>
            <Textarea
              className="min-h-16 resize-none"
              disabled={isPending}
              id="template-description"
              onChange={(e) => setDescription(e.target.value)}
              placeholder="What this template is for"
              rows={2}
              value={description}
            />
          </div>

          {/* Canvas Template */}
          <div className="space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="template-canvas"
            >
              Canvas template{" "}
              <span className="font-normal text-muted-foreground">
                (optional)
              </span>
            </label>
            <Textarea
              className="min-h-20 resize-none font-mono text-xs"
              disabled={isPending}
              id="template-canvas"
              onChange={(e) => setCanvasTemplate(e.target.value)}
              placeholder="Canvas content here..."
              rows={4}
              value={canvasTemplate}
            />
            <p className="text-xs text-muted-foreground">
              Use {"{channel.name}"} and {"{template.name}"} as placeholders.
            </p>
          </div>

          {/* Agent Personas */}
          <AddChannelBotPersonasSection
            canToggleSelections={!isPending}
            includeGeneric={false}
            isLoading={personasQuery.isLoading}
            onToggleGeneric={() => {}}
            onTogglePersona={handleTogglePersona}
            personas={personasQuery.data ?? []}
            selectedPersonaIds={selectedPersonaIds}
            showGeneric={false}
          />

          {/* Agent Teams */}
          <TemplateTeamSelector
            isPending={isPending}
            onToggleTeam={handleToggleTeam}
            selectedTeamIds={selectedTeamIds}
            teams={teamsQuery.data ?? []}
            isLoading={teamsQuery.isLoading}
          />

          {/* Runtime assignments */}
          <RuntimeAssignments
            isPending={isPending}
            personas={personasQuery.data ?? []}
            personaRuntimes={personaRuntimes}
            providers={runtimes}
            providersLoading={providersQuery.isLoading}
            selectedPersonaIds={selectedPersonaIds}
            selectedTeamIds={selectedTeamIds}
            teamRuntimes={teamRuntimes}
            teams={teamsQuery.data ?? []}
            onPersonaRuntimeChange={(personaId, runtimeId) =>
              setPersonaRuntimes((prev) => ({
                ...prev,
                [personaId]: runtimeId,
              }))
            }
            onTeamRuntimeChange={(teamId, runtimeId) =>
              setTeamRuntimes((prev) => ({ ...prev, [teamId]: runtimeId }))
            }
          />
        </form>
      </ChooserDialogContent>
    </Dialog>
  );
}

function TemplateTeamSelector({
  isPending,
  isLoading,
  onToggleTeam,
  selectedTeamIds,
  teams,
}: {
  isPending: boolean;
  isLoading: boolean;
  onToggleTeam: (teamId: string) => void;
  selectedTeamIds: readonly string[];
  teams: readonly { id: string; name: string }[];
}) {
  if (isLoading || teams.length === 0) {
    return null;
  }

  return (
    <div className="space-y-3">
      <div>
        <div className="text-sm font-medium">Teams</div>
        <p className="text-sm font-normal text-muted-foreground">
          Select teams to include in this template.
        </p>
      </div>
      <div className="flex flex-wrap gap-2">
        {teams.map((team) => {
          const isSelected = selectedTeamIds.includes(team.id);
          return (
            <button
              key={team.id}
              type="button"
              aria-pressed={isSelected}
              className={cn(
                "inline-flex min-h-9 items-center gap-2 rounded-full border px-3 py-1.5 text-sm font-medium transition-colors focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
                isSelected
                  ? "border-primary bg-primary/10 text-foreground"
                  : "border-border/80 bg-background/60 text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                isPending && "cursor-not-allowed opacity-50",
              )}
              disabled={isPending}
              onClick={() => onToggleTeam(team.id)}
            >
              <Users
                className={cn(
                  "h-4 w-4",
                  isSelected ? "text-primary" : "text-current",
                )}
              />
              {team.name}
            </button>
          );
        })}
      </div>
    </div>
  );
}

function RuntimeAssignments({
  isPending,
  onPersonaRuntimeChange,
  onTeamRuntimeChange,
  personas,
  personaRuntimes,
  providers,
  providersLoading,
  selectedPersonaIds,
  selectedTeamIds,
  teamRuntimes,
  teams,
}: {
  isPending: boolean;
  onPersonaRuntimeChange: (personaId: string, runtimeId: string) => void;
  onTeamRuntimeChange: (teamId: string, runtimeId: string) => void;
  personas: AgentPersona[];
  personaRuntimes: Record<string, string>;
  providers: AcpRuntime[];
  providersLoading: boolean;
  selectedPersonaIds: readonly string[];
  selectedTeamIds: readonly string[];
  teamRuntimes: Record<string, string>;
  teams: readonly AgentTeam[];
}) {
  const hasSelections =
    selectedPersonaIds.length > 0 || selectedTeamIds.length > 0;
  if (!hasSelections) return null;

  const selectedPersonas = personas.filter((p) =>
    selectedPersonaIds.includes(p.id),
  );
  const selectedTeams = teams.filter((t) => selectedTeamIds.includes(t.id));

  return (
    <div className="space-y-3">
      <div>
        <div className="text-sm font-medium">Runtimes</div>
        <p className="text-sm font-normal text-muted-foreground">
          Choose which runtime to use for each agent.
        </p>
      </div>

      {providersLoading ? (
        <p className="text-xs text-muted-foreground">Discovering runtimes...</p>
      ) : providers.length === 0 ? (
        <p className="text-xs text-muted-foreground">
          No ACP runtimes detected. Install one to assign runtimes.
        </p>
      ) : (
        <div className="space-y-2">
          {selectedPersonas.map((persona) => (
            <RuntimeRow
              key={persona.id}
              avatarUrl={persona.avatarUrl}
              disabled={isPending}
              label={persona.displayName}
              onChange={(runtimeId) =>
                onPersonaRuntimeChange(persona.id, runtimeId)
              }
              providers={providers}
              value={personaRuntimes[persona.id] ?? ""}
            />
          ))}
          {selectedTeams.map((team) => (
            <RuntimeRow
              key={team.id}
              disabled={isPending}
              icon="team"
              label={team.name}
              onChange={(runtimeId) => onTeamRuntimeChange(team.id, runtimeId)}
              providers={providers}
              value={teamRuntimes[team.id] ?? ""}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function RuntimeRow({
  avatarUrl,
  disabled,
  icon,
  label,
  onChange,
  providers,
  value,
}: {
  avatarUrl?: string | null | undefined;
  disabled: boolean;
  icon?: "team";
  label: string;
  onChange: (runtimeId: string) => void;
  providers: AcpRuntime[];
  value: string;
}) {
  return (
    <div className="flex items-center gap-2">
      <div className="flex min-w-0 flex-1 items-center gap-2">
        {icon === "team" ? (
          <Users className="h-4 w-4 shrink-0 text-muted-foreground" />
        ) : (
          <ProfileAvatar
            avatarUrl={avatarUrl ?? null}
            className="h-5 w-5 shrink-0 text-3xs bg-muted text-muted-foreground ring-1 ring-border/50"
            label={label}
          />
        )}
        <span className="truncate text-sm">{label}</span>
      </div>
      <select
        className="h-7 rounded-md border border-input bg-background px-2 text-xs shadow-xs focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
        disabled={disabled}
        onChange={(e) => onChange(e.target.value)}
        value={value}
      >
        <option value="">Default</option>
        {providers.map((runtime) => (
          <option key={runtime.id} value={runtime.id}>
            {runtime.label}
          </option>
        ))}
      </select>
    </div>
  );
}
