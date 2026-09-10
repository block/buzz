import * as React from "react";

import type { BwIssueTemplate } from "@/features/projects/bwIssueTemplates";
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

export type CreateProjectWorkItemDialogInput = {
  title: string;
  body: string;
  acceptanceCriteria?: string[];
  nonGoals?: string[];
};

function splitLines(value: string): string[] {
  return value
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

export function CreateProjectWorkItemDialog({
  bodyPlaceholder,
  children,
  description,
  isCreating,
  itemName,
  onCreate,
  onOpenChange,
  open,
  submitDisabled = false,
  templates,
  title,
  titlePlaceholder,
}: {
  bodyPlaceholder: string;
  children?: React.ReactNode;
  description: string;
  isCreating: boolean;
  itemName: "issue" | "pull-request";
  onCreate: (input: CreateProjectWorkItemDialogInput) => Promise<void>;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  submitDisabled?: boolean;
  /** BW-fixed bug/feature/operations templates. When supplied, the dialog
   * additionally requires at least one acceptance criterion before submit —
   * this is the same small fixed set for every repository, never a custom
   * per-project template engine. */
  templates?: BwIssueTemplate[];
  title: string;
  titlePlaceholder: string;
}) {
  const [workItemTitle, setWorkItemTitle] = React.useState("");
  const [body, setBody] = React.useState("");
  const [templateId, setTemplateId] = React.useState("");
  const [acceptanceCriteria, setAcceptanceCriteria] = React.useState("");
  const [nonGoals, setNonGoals] = React.useState("");
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);
  const titleInputRef = React.useRef<HTMLInputElement>(null);
  const submitInFlightRef = React.useRef(false);
  const testIdPrefix = `create-${itemName}`;
  const itemLabel = itemName === "issue" ? "issue" : "pull request";
  const criteriaRequired = Boolean(templates);
  const criteriaFilled = splitLines(acceptanceCriteria).length > 0;

  React.useEffect(() => {
    if (!open) return;
    setWorkItemTitle("");
    setBody("");
    setTemplateId("");
    setAcceptanceCriteria("");
    setNonGoals("");
    setErrorMessage(null);
    const timerId = globalThis.setTimeout(
      () => titleInputRef.current?.focus(),
      50,
    );
    return () => globalThis.clearTimeout(timerId);
  }, [open]);

  function handleTemplateSelect(id: string) {
    setTemplateId(id);
    const template = templates?.find((candidate) => candidate.id === id);
    if (!template) return;
    setBody(template.descriptionSkeleton);
    setAcceptanceCriteria(template.defaultAcceptanceCriteria.join("\n"));
    setNonGoals(template.defaultNonGoals.join("\n"));
  }

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (isCreating || submitDisabled || submitInFlightRef.current) return;
    const trimmedTitle = workItemTitle.trim();
    if (!trimmedTitle) return;
    if (criteriaRequired && !criteriaFilled) {
      setErrorMessage("At least one acceptance criterion is required.");
      return;
    }
    submitInFlightRef.current = true;
    setErrorMessage(null);
    try {
      await onCreate({
        acceptanceCriteria: templates
          ? splitLines(acceptanceCriteria)
          : undefined,
        body: body.trim(),
        nonGoals: templates ? splitLines(nonGoals) : undefined,
        title: trimmedTitle,
      });
      onOpenChange(false);
    } catch (error) {
      setErrorMessage(
        error instanceof Error
          ? error.message
          : `Failed to create ${itemLabel}.`,
      );
    } finally {
      submitInFlightRef.current = false;
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
        data-testid={`${testIdPrefix}-dialog`}
        description={description}
        footer={
          <div className="flex w-full justify-end">
            <Button
              data-testid={`${testIdPrefix}-submit`}
              disabled={
                isCreating ||
                submitDisabled ||
                workItemTitle.trim().length === 0 ||
                (criteriaRequired && !criteriaFilled)
              }
              form={`${testIdPrefix}-form`}
              type="submit"
            >
              {isCreating ? "Creating…" : `Create ${itemLabel}`}
            </Button>
          </div>
        }
        footerClassName="border-t-0 pt-0"
        headerClassName="pb-2"
        title={title}
      >
        <form
          className="space-y-5"
          id={`${testIdPrefix}-form`}
          onSubmit={(event) => void handleSubmit(event)}
        >
          {children}
          <div className="space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor={`${testIdPrefix}-title`}
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
                data-testid={`${testIdPrefix}-title`}
                disabled={isCreating}
                id={`${testIdPrefix}-title`}
                maxLength={256}
                onChange={(event) => {
                  setWorkItemTitle(event.target.value);
                  setErrorMessage(null);
                }}
                placeholder={titlePlaceholder}
                ref={titleInputRef}
                value={workItemTitle}
              />
            </div>
          </div>
          <div className="space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor={`${testIdPrefix}-body`}
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
                data-testid={`${testIdPrefix}-body`}
                disabled={isCreating}
                id={`${testIdPrefix}-body`}
                onChange={(event) => {
                  setBody(event.target.value);
                  setErrorMessage(null);
                }}
                placeholder={bodyPlaceholder}
                value={body}
              />
            </div>
          </div>
          {templates ? (
            <>
              <div className="space-y-1.5">
                <label
                  className="text-sm font-medium text-foreground"
                  htmlFor={`${testIdPrefix}-template`}
                >
                  Template
                </label>
                <select
                  className="h-9 w-full rounded-lg border border-input bg-background px-2 text-sm outline-hidden focus:ring-1 focus:ring-ring"
                  data-testid={`${testIdPrefix}-template`}
                  disabled={isCreating}
                  id={`${testIdPrefix}-template`}
                  onChange={(event) => handleTemplateSelect(event.target.value)}
                  value={templateId}
                >
                  <option value="">No template</option>
                  {templates.map((template) => (
                    <option key={template.id} value={template.id}>
                      {template.label}
                    </option>
                  ))}
                </select>
              </div>
              <div className="space-y-1.5">
                <label
                  className="text-sm font-medium text-foreground"
                  htmlFor={`${testIdPrefix}-acceptance-criteria`}
                >
                  Acceptance criteria
                  <span className="ml-1 text-xs font-normal text-muted-foreground/50">
                    One per line, at least one required
                  </span>
                </label>
                <div className={FIELD_SHELL_CLASS}>
                  <Textarea
                    className={cn(
                      "min-h-16 resize-y px-3 py-2",
                      FIELD_CONTROL_CLASS,
                    )}
                    data-testid={`${testIdPrefix}-acceptance-criteria`}
                    disabled={isCreating}
                    id={`${testIdPrefix}-acceptance-criteria`}
                    onChange={(event) => {
                      setAcceptanceCriteria(event.target.value);
                      setErrorMessage(null);
                    }}
                    value={acceptanceCriteria}
                  />
                </div>
              </div>
              <div className="space-y-1.5">
                <label
                  className="text-sm font-medium text-foreground"
                  htmlFor={`${testIdPrefix}-non-goals`}
                >
                  Non-goals
                  <span className="ml-1 text-xs font-normal text-muted-foreground/50">
                    Optional, one per line
                  </span>
                </label>
                <div className={FIELD_SHELL_CLASS}>
                  <Textarea
                    className={cn(
                      "min-h-12 resize-y px-3 py-2",
                      FIELD_CONTROL_CLASS,
                    )}
                    data-testid={`${testIdPrefix}-non-goals`}
                    disabled={isCreating}
                    id={`${testIdPrefix}-non-goals`}
                    onChange={(event) => setNonGoals(event.target.value)}
                    value={nonGoals}
                  />
                </div>
              </div>
            </>
          ) : null}
          {errorMessage ? (
            <p className="text-sm text-destructive">{errorMessage}</p>
          ) : null}
        </form>
      </ChooserDialogContent>
    </Dialog>
  );
}
