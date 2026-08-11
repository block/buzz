import * as React from "react";

import type { Project } from "@/features/projects/hooks";
import type { AddProjectRepositoryInput } from "@/features/projects/useAddProjectRepository";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";

const FIELD_SHELL_CLASS =
  "flex min-h-11 items-center rounded-xl border border-input bg-muted/40 px-3 transition-colors hover:border-muted-foreground/40 focus-within:border-muted-foreground/50";
const FIELD_CONTROL_CLASS =
  "h-8 border-0 bg-transparent px-0 py-0 text-muted-foreground/55 shadow-none outline-none ring-0 placeholder:text-muted-foreground/55 focus:bg-transparent focus:text-foreground focus-visible:ring-0";

export function AddProjectRepositoryDialog({
  accessChannelId,
  channels,
  isCreating,
  onAdd,
  onOpenChange,
  open,
  project,
}: {
  accessChannelId?: string;
  channels: Channel[];
  isCreating: boolean;
  onAdd: (input: AddProjectRepositoryInput) => Promise<void>;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  project: Project;
}) {
  const [name, setName] = React.useState("");
  const [cloneUrl, setCloneUrl] = React.useState("");
  const [selectedChannelId, setSelectedChannelId] = React.useState("");
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);
  const nameInputRef = React.useRef<HTMLInputElement>(null);

  React.useEffect(() => {
    if (!open) return;
    setName("");
    setCloneUrl("");
    setSelectedChannelId(accessChannelId ?? "");
    setErrorMessage(null);
    const timerId = globalThis.setTimeout(
      () => nameInputRef.current?.focus(),
      50,
    );
    return () => globalThis.clearTimeout(timerId);
  }, [accessChannelId, open]);

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!name.trim() || !selectedChannelId) return;
    setErrorMessage(null);
    try {
      await onAdd({
        accessChannelId: selectedChannelId,
        cloneUrl: cloneUrl.trim() || undefined,
        name: name.trim(),
        project,
      });
      onOpenChange(false);
    } catch (error) {
      setErrorMessage(
        error instanceof Error ? error.message : "Failed to add repository.",
      );
    }
  }

  return (
    <Dialog
      onOpenChange={(nextOpen) => {
        if (!nextOpen && isCreating) return;
        onOpenChange(nextOpen);
      }}
      open={open}
    >
      <ChooserDialogContent
        className="max-w-lg"
        contentClassName="pt-3"
        data-testid="add-project-repository-dialog"
        description={`Add another repository to ${project.name}.`}
        footer={
          <Button
            data-testid="add-project-repository-submit"
            disabled={isCreating || !name.trim() || !selectedChannelId}
            form="add-project-repository-form"
            type="submit"
          >
            {isCreating ? "Adding..." : "Add repository"}
          </Button>
        }
        footerClassName="border-t-0 pt-0"
        headerClassName="pb-2"
        title="Add repository"
      >
        <form
          className="space-y-5"
          id="add-project-repository-form"
          onSubmit={(event) => void handleSubmit(event)}
        >
          <div className="space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="add-project-repository-name"
            >
              Name
            </label>
            <div className={FIELD_SHELL_CLASS}>
              <Input
                autoCapitalize="none"
                autoComplete="off"
                autoCorrect="off"
                className={cn(FIELD_CONTROL_CLASS)}
                data-testid="add-project-repository-name"
                disabled={isCreating}
                id="add-project-repository-name"
                onChange={(event) => {
                  setName(event.target.value);
                  setErrorMessage(null);
                }}
                placeholder="mobile-app"
                ref={nameInputRef}
                spellCheck={false}
                value={name}
              />
            </div>
          </div>
          <div className="space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="add-project-repository-channel"
            >
              Access channel
            </label>
            <div className={FIELD_SHELL_CLASS}>
              <select
                className={cn(FIELD_CONTROL_CLASS, "w-full")}
                data-testid="add-project-repository-channel"
                disabled={isCreating}
                id="add-project-repository-channel"
                onChange={(event) => {
                  setSelectedChannelId(event.target.value);
                  setErrorMessage(null);
                }}
                required
                value={selectedChannelId}
              >
                <option value="">Select a channel</option>
                {channels.map((channel) => (
                  <option key={channel.id} value={channel.id}>
                    {channel.name}
                  </option>
                ))}
              </select>
            </div>
            <p className="text-xs text-muted-foreground">
              Members of this channel can access the repository.
            </p>
          </div>
          <div className="space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="add-project-repository-clone-url"
            >
              Clone URL
              <span className="ml-1 text-xs font-normal text-muted-foreground/50">
                Optional
              </span>
            </label>
            <div className={FIELD_SHELL_CLASS}>
              <Input
                autoCapitalize="none"
                autoComplete="off"
                autoCorrect="off"
                className={cn(FIELD_CONTROL_CLASS)}
                data-testid="add-project-repository-clone-url"
                disabled={isCreating}
                id="add-project-repository-clone-url"
                onChange={(event) => {
                  setCloneUrl(event.target.value);
                  setErrorMessage(null);
                }}
                placeholder="https://relay.example.com/git/mobile-app.git"
                spellCheck={false}
                value={cloneUrl}
              />
            </div>
          </div>
          {errorMessage ? (
            <p className="text-sm text-destructive">{errorMessage}</p>
          ) : null}
        </form>
      </ChooserDialogContent>
    </Dialog>
  );
}
