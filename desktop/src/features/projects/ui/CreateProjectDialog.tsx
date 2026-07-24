import * as React from "react";

import type { CreateProjectInput } from "@/features/projects/useCreateProject";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";

const CREATE_FIELD_SHELL_CLASS =
  "rounded-xl border border-input bg-muted/40 transition-colors duration-150 ease-out hover:border-muted-foreground/40 focus-within:border-muted-foreground/50";
const CREATE_FIELD_CONTROL_CLASS =
  "border-0 bg-transparent text-muted-foreground/55 shadow-none outline-none ring-0 transition-colors duration-150 ease-out placeholder:text-muted-foreground/55 focus:bg-transparent focus:text-foreground focus:outline-hidden focus-visible:ring-0";
const CREATE_LABEL_OPTIONAL_CLASS =
  "ml-1 text-xs font-normal text-muted-foreground/50";

type CreateProjectDialogProps = {
  isCreating: boolean;
  onCreate: (input: CreateProjectInput) => Promise<void>;
  onOpenChange: (open: boolean) => void;
  open: boolean;
};

/** Modal for publishing a new project (NIP-34 repo announcement). */
export function CreateProjectDialog({
  isCreating,
  onCreate,
  onOpenChange,
  open,
}: CreateProjectDialogProps) {
  const [name, setName] = React.useState("");
  const [description, setDescription] = React.useState("");
  const [cloneUrl, setCloneUrl] = React.useState("");
  const [webUrl, setWebUrl] = React.useState("");
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);
  const nameInputRef = React.useRef<HTMLInputElement>(null);

  React.useEffect(() => {
    if (!open) return;

    setName("");
    setDescription("");
    setCloneUrl("");
    setWebUrl("");
    setErrorMessage(null);

    // Small delay to let the dialog animation start before focusing.
    const timerId = globalThis.setTimeout(() => {
      nameInputRef.current?.focus();
    }, 50);
    return () => globalThis.clearTimeout(timerId);
  }, [open]);

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();

    const trimmedName = name.trim();
    if (!trimmedName) return;

    setErrorMessage(null);

    try {
      await onCreate({
        name: trimmedName,
        description: description.trim() || undefined,
        cloneUrl: cloneUrl.trim() || undefined,
        webUrl: webUrl.trim() || undefined,
      });

      onOpenChange(false);
    } catch (error) {
      setErrorMessage(
        error instanceof Error ? error.message : "Failed to create project.",
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
        data-testid="create-project-dialog"
        description="Projects are repositories published to this workspace's relay."
        footer={
          <div className="flex w-full items-center justify-end gap-3">
            <Button
              data-testid="create-project-submit"
              disabled={isCreating || name.trim().length === 0}
              form="create-project-form"
              type="submit"
            >
              {isCreating ? "Creating..." : "Create project"}
            </Button>
          </div>
        }
        footerClassName="border-t-0 pt-0"
        headerClassName="pb-2"
        title="Create a new project"
      >
        <form
          className="space-y-5"
          id="create-project-form"
          onSubmit={(event) => {
            void handleSubmit(event);
          }}
        >
          <div className="space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="create-project-name"
            >
              Name
            </label>
            <div
              className={cn(
                "flex min-h-11 items-center px-3",
                CREATE_FIELD_SHELL_CLASS,
              )}
            >
              <Input
                autoCapitalize="none"
                autoComplete="off"
                autoCorrect="off"
                className={cn(
                  "h-8 px-0 py-0 leading-6",
                  CREATE_FIELD_CONTROL_CLASS,
                )}
                data-testid="create-project-name"
                disabled={isCreating}
                id="create-project-name"
                onChange={(event) => {
                  setName(event.target.value);
                  setErrorMessage(null);
                }}
                placeholder="bee-garden-game"
                ref={nameInputRef}
                spellCheck={false}
                value={name}
              />
            </div>
          </div>

          <div className="space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="create-project-description"
            >
              Description
              <span className={CREATE_LABEL_OPTIONAL_CLASS}>Optional</span>
            </label>
            <div className={CREATE_FIELD_SHELL_CLASS}>
              <Textarea
                className={cn(
                  "min-h-20 resize-none px-3 py-3 leading-5",
                  CREATE_FIELD_CONTROL_CLASS,
                )}
                data-testid="create-project-description"
                disabled={isCreating}
                id="create-project-description"
                onChange={(event) => {
                  setDescription(event.target.value);
                  setErrorMessage(null);
                }}
                placeholder="What this project is about"
                rows={2}
                value={description}
              />
            </div>
          </div>

          <div className="space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="create-project-clone-url"
            >
              Clone URL
              <span className={CREATE_LABEL_OPTIONAL_CLASS}>Optional</span>
            </label>
            <div
              className={cn(
                "flex min-h-11 items-center px-3",
                CREATE_FIELD_SHELL_CLASS,
              )}
            >
              <Input
                autoCapitalize="none"
                autoComplete="off"
                autoCorrect="off"
                className={cn(
                  "h-8 px-0 py-0 leading-6",
                  CREATE_FIELD_CONTROL_CLASS,
                )}
                data-testid="create-project-clone-url"
                disabled={isCreating}
                id="create-project-clone-url"
                onChange={(event) => {
                  setCloneUrl(event.target.value);
                  setErrorMessage(null);
                }}
                placeholder="https://relay.example.com/git/bee-garden-game.git"
                spellCheck={false}
                value={cloneUrl}
              />
            </div>
          </div>

          <div className="space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="create-project-web-url"
            >
              Web URL
              <span className={CREATE_LABEL_OPTIONAL_CLASS}>Optional</span>
            </label>
            <div
              className={cn(
                "flex min-h-11 items-center px-3",
                CREATE_FIELD_SHELL_CLASS,
              )}
            >
              <Input
                autoCapitalize="none"
                autoComplete="off"
                autoCorrect="off"
                className={cn(
                  "h-8 px-0 py-0 leading-6",
                  CREATE_FIELD_CONTROL_CLASS,
                )}
                data-testid="create-project-web-url"
                disabled={isCreating}
                id="create-project-web-url"
                onChange={(event) => {
                  setWebUrl(event.target.value);
                  setErrorMessage(null);
                }}
                placeholder="https://github.com/owner/repo"
                spellCheck={false}
                value={webUrl}
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
