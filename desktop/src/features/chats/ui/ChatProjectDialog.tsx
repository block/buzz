import * as React from "react";
import { FolderOpen, Sparkles } from "lucide-react";
import { toast } from "sonner";

import {
  type ChatProject,
  makeChatProjectId,
} from "@/features/chats/lib/chatSetup";
import { pickChatProjectDirectory } from "@/shared/api/tauriChats";
import type { ChannelTemplate } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Spinner } from "@/shared/ui/spinner";

type ChatProjectDialogProps = {
  mode?: "create" | "edit";
  onOpenChange: (open: boolean) => void;
  onSaveProject: (project: ChatProject) => void;
  open: boolean;
  project?: ChatProject | null;
  templates: ChannelTemplate[];
};

function errorMessage(error: unknown) {
  if (error instanceof Error && error.message.trim()) {
    return error.message.trim();
  }
  if (typeof error === "string" && error.trim()) {
    return error.trim();
  }
  return "Unknown error";
}

export function ChatProjectDialog({
  mode = "create",
  onOpenChange,
  onSaveProject,
  open,
  project = null,
  templates,
}: ChatProjectDialogProps) {
  const [name, setName] = React.useState("");
  const [path, setPath] = React.useState("");
  const [templateId, setTemplateId] = React.useState("");
  const [isChoosingFolder, setIsChoosingFolder] = React.useState(false);
  const selectedTemplate =
    templates.find((template) => template.id === templateId) ?? null;
  const canSave = name.trim().length > 0;

  React.useEffect(() => {
    if (!open) {
      return;
    }
    setName(project?.name ?? "");
    setPath(project?.path ?? "");
    setTemplateId(project?.templateId ?? "");
  }, [open, project]);

  const handleSave = React.useCallback(() => {
    if (!canSave) {
      return;
    }
    onSaveProject({
      id: project?.id ?? makeChatProjectId(),
      name: name.trim(),
      path: path.trim() || null,
      templateId: templateId || null,
      updatedAt: Math.floor(Date.now() / 1_000),
      chatCount: project?.chatCount ?? 0,
    });
    onOpenChange(false);
  }, [canSave, name, onOpenChange, onSaveProject, path, project, templateId]);

  const handleChooseFolder = React.useCallback(async () => {
    setIsChoosingFolder(true);
    try {
      const selectedPath = await pickChatProjectDirectory();
      if (!selectedPath) {
        return;
      }
      setPath(selectedPath);
      if (!name.trim()) {
        setName(projectNameFromPath(selectedPath));
      }
    } catch (error) {
      console.error("Failed to choose project folder", error);
      toast.error("Could not choose folder", {
        description: errorMessage(error),
      });
    } finally {
      setIsChoosingFolder(false);
    }
  }, [name]);

  const isEdit = mode === "edit";

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>
            {isEdit ? "Project settings" : "Add new project"}
          </DialogTitle>
          <DialogDescription>
            {isEdit
              ? "Change the name, folder, and default template for new chats."
              : "Name the work and choose the folder Fizz should treat as its home."}
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <div className="space-y-1.5">
            <label className="text-sm font-medium" htmlFor="chat-project-name">
              What are you working on?
            </label>
            <Input
              autoFocus
              id="chat-project-name"
              onChange={(event) => setName(event.target.value)}
              placeholder="Project name"
              value={name}
            />
          </div>
          <div className="space-y-1.5">
            <label className="text-sm font-medium" htmlFor="chat-project-path">
              Folder
            </label>
            <div className="flex gap-2">
              <Input
                id="chat-project-path"
                onChange={(event) => setPath(event.target.value)}
                placeholder="/Users/me/Development/project"
                value={path}
              />
              <Button
                disabled={isChoosingFolder}
                onClick={() => void handleChooseFolder()}
                type="button"
                variant="secondary"
              >
                {isChoosingFolder ? (
                  <Spinner className="h-4 w-4" />
                ) : (
                  <FolderOpen className="h-4 w-4" />
                )}
                Choose
              </Button>
            </div>
          </div>
          <div className="space-y-1.5">
            <div className="text-sm font-medium">Use template</div>
            <div className="rounded-lg border border-border/60 p-1">
              <ProjectTemplateRow
                checked={!selectedTemplate}
                label="No template"
                onSelect={() => setTemplateId("")}
              />
              {templates.map((template) => (
                <ProjectTemplateRow
                  checked={selectedTemplate?.id === template.id}
                  key={template.id}
                  label={template.name}
                  onSelect={() => setTemplateId(template.id)}
                />
              ))}
            </div>
          </div>
        </div>
        <DialogFooter>
          <Button
            onClick={() => onOpenChange(false)}
            type="button"
            variant="ghost"
          >
            Cancel
          </Button>
          <Button disabled={!canSave} onClick={handleSave} type="button">
            {isEdit ? "Save changes" : "Create project"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function ProjectTemplateRow({
  checked,
  label,
  onSelect,
}: {
  checked: boolean;
  label: string;
  onSelect: () => void;
}) {
  return (
    <button
      className="flex w-full items-center gap-3 rounded-lg px-2 py-2 text-left text-sm outline-hidden transition-colors hover:bg-muted/60 focus-visible:bg-muted/60"
      onClick={onSelect}
      type="button"
    >
      <Sparkles className="h-4 w-4" />
      <span className="min-w-0 flex-1 truncate font-medium">{label}</span>
      {checked ? <span className="h-2 w-2 rounded-full bg-primary" /> : null}
    </button>
  );
}

function projectNameFromPath(path: string) {
  const normalized = path.trim().replace(/[\\/]+$/, "");
  return normalized.split(/[\\/]/).filter(Boolean).pop() ?? "New project";
}
