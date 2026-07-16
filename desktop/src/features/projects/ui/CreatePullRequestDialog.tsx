import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";

const FIELD_SHELL_CLASS =
  "rounded-xl border border-input bg-muted/40 transition-colors hover:border-muted-foreground/40 focus-within:border-muted-foreground/50";
const FIELD_CONTROL_CLASS =
  "border-0 bg-transparent shadow-none outline-none ring-0 placeholder:text-muted-foreground/55 focus-visible:ring-0";

export type CreatePullRequestDialogInput = {
  title: string;
  body: string;
};

export function CreatePullRequestDialog({
  commit,
  isCreating,
  onCreate,
  onOpenChange,
  open,
  sourceBranch,
  targetBranch,
}: {
  commit: string;
  isCreating: boolean;
  onCreate: (input: CreatePullRequestDialogInput) => Promise<void>;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  sourceBranch: string;
  targetBranch: string;
}) {
  const [title, setTitle] = React.useState("");
  const [body, setBody] = React.useState("");
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);
  const titleInputRef = React.useRef<HTMLInputElement>(null);

  React.useEffect(() => {
    if (!open) return;
    setTitle("");
    setBody("");
    setErrorMessage(null);
    const timerId = globalThis.setTimeout(
      () => titleInputRef.current?.focus(),
      50,
    );
    return () => globalThis.clearTimeout(timerId);
  }, [open]);

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmedTitle = title.trim();
    if (!trimmedTitle) return;
    setErrorMessage(null);
    try {
      await onCreate({ title: trimmedTitle, body: body.trim() });
      onOpenChange(false);
    } catch (error) {
      setErrorMessage(
        error instanceof Error
          ? error.message
          : "Failed to create pull request.",
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
        data-testid="create-pull-request-dialog"
        description={`${sourceBranch} → ${targetBranch} at ${commit.slice(0, 7)}`}
        footer={
          <div className="flex w-full justify-end">
            <Button
              data-testid="create-pull-request-submit"
              disabled={isCreating || title.trim().length === 0}
              form="create-pull-request-form"
              type="submit"
            >
              {isCreating ? "Creating…" : "Create pull request"}
            </Button>
          </div>
        }
        footerClassName="border-t-0 pt-0"
        headerClassName="pb-2"
        title="Open a pull request"
      >
        <form
          className="space-y-5"
          id="create-pull-request-form"
          onSubmit={(event) => void handleSubmit(event)}
        >
          <div className="space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="create-pull-request-title"
            >
              Title
            </label>
            <div
              className={cn(
                "flex min-h-11 items-center px-3",
                FIELD_SHELL_CLASS,
              )}
            >
              <Input
                className={cn("h-8 px-0", FIELD_CONTROL_CLASS)}
                data-testid="create-pull-request-title"
                disabled={isCreating}
                id="create-pull-request-title"
                maxLength={256}
                onChange={(event) => {
                  setTitle(event.target.value);
                  setErrorMessage(null);
                }}
                placeholder="Describe the change"
                ref={titleInputRef}
                value={title}
              />
            </div>
          </div>
          <div className="space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="create-pull-request-body"
            >
              Description
              <span className="ml-1 text-xs font-normal text-muted-foreground/50">
                Optional
              </span>
            </label>
            <div className={FIELD_SHELL_CLASS}>
              <Textarea
                className={cn(
                  "min-h-28 resize-y px-3 py-3",
                  FIELD_CONTROL_CLASS,
                )}
                data-testid="create-pull-request-body"
                disabled={isCreating}
                id="create-pull-request-body"
                onChange={(event) => {
                  setBody(event.target.value);
                  setErrorMessage(null);
                }}
                placeholder="Add context for reviewers"
                value={body}
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
