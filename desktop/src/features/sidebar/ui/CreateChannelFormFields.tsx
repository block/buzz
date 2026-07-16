import { ChevronDown, ClockFading, Hash } from "lucide-react";

import type { ChannelTemplate } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Switch } from "@/shared/ui/switch";
import { Textarea } from "@/shared/ui/textarea";

import type { CreateChannelFormState } from "@/features/sidebar/lib/useCreateChannelForm";

const CREATE_FIELD_SHELL_CLASS =
  "rounded-xl border border-input bg-muted/40 transition-colors duration-150 ease-out hover:border-muted-foreground/40 focus-within:border-muted-foreground/50";
const CREATE_FIELD_CONTROL_CLASS =
  "border-0 bg-transparent text-muted-foreground/55 shadow-none outline-none ring-0 transition-colors duration-150 ease-out placeholder:text-muted-foreground/55 focus:bg-transparent focus:text-foreground focus:outline-hidden focus-visible:ring-0";
const CREATE_LABEL_OPTIONAL_CLASS =
  "ml-1 text-xs font-normal text-muted-foreground/50";

export const CREATE_CHANNEL_FORM_ID = "create-channel-form";

/**
 * The body of the create-channel form (name, description, private toggle,
 * optional template). Rendered inside both the standalone dialog and the
 * "Add channel" browser's create mode. Wrap in a `<form>` with
 * `id={CREATE_CHANNEL_FORM_ID}` and hook up `form.handleSubmit`.
 */
export function CreateChannelFormFields({
  form,
}: {
  form: CreateChannelFormState;
}) {
  const { channelKind, kindLabel, isCreating } = form;

  return (
    <div className="space-y-5">
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="create-channel-name"
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
            data-testid="create-channel-name"
            disabled={isCreating}
            id="create-channel-name"
            onChange={(event) => form.setName(event.target.value)}
            placeholder={
              channelKind === "forum" ? "design-discussions" : "release-notes"
            }
            ref={form.nameInputRef}
            spellCheck={false}
            value={form.name}
          />
        </div>
      </div>

      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="create-channel-description"
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
            data-testid="create-channel-description"
            disabled={isCreating}
            id="create-channel-description"
            onChange={(event) => form.setDescription(event.target.value)}
            placeholder={`What this ${kindLabel} is for`}
            rows={2}
            value={form.description}
          />
        </div>
      </div>

      <div
        className={cn(
          "flex min-h-12 items-center justify-between gap-4 rounded-xl py-1",
          isCreating && "opacity-50",
        )}
        data-testid="create-channel-visibility"
      >
        <label
          className="min-w-0 cursor-pointer space-y-0.5"
          htmlFor="create-channel-private"
        >
          <span className="block text-sm font-medium text-foreground">
            Private
          </span>
          <span
            className="block text-xs leading-4 text-muted-foreground/65"
            id="create-channel-private-description"
          >
            Only members can invite people to this {kindLabel}.
          </span>
        </label>
        <Switch
          aria-describedby="create-channel-private-description"
          checked={form.visibility === "private"}
          className="shrink-0 shadow-none [&>span]:shadow-none"
          data-testid="create-channel-private-toggle"
          disabled={isCreating}
          id="create-channel-private"
          onCheckedChange={(checked) =>
            form.setVisibility(checked ? "private" : "open")
          }
        />
      </div>

      {form.templates.length > 0 ? (
        <div className="space-y-1.5">
          <label
            className="text-sm font-medium text-foreground"
            htmlFor="create-channel-template"
          >
            Template
            <span className={CREATE_LABEL_OPTIONAL_CLASS}>Optional</span>
          </label>
          <select
            className="flex min-h-11 w-full rounded-xl border border-input bg-muted/40 px-3 py-2 text-sm text-muted-foreground/55 shadow-none transition-colors duration-150 ease-out hover:border-muted-foreground/40 focus:border-muted-foreground/50 focus:text-foreground focus-visible:outline-hidden disabled:cursor-not-allowed disabled:opacity-50"
            data-testid="create-channel-template"
            disabled={isCreating}
            id="create-channel-template"
            onChange={(event) => form.handleTemplateChange(event.target.value)}
            value={form.selectedTemplateId ?? ""}
          >
            <option value="">No template</option>
            {form.templates.map((template: ChannelTemplate) => (
              <option key={template.id} value={template.id}>
                {template.name}
              </option>
            ))}
          </select>
        </div>
      ) : null}

      {form.errorMessage ? (
        <p className="text-sm text-destructive">{form.errorMessage}</p>
      ) : null}
    </div>
  );
}

/**
 * Footer for the create-channel form: the Ongoing/Temporary duration picker on
 * the left and the submit button on the right. The submit button is bound to
 * the form via `form={CREATE_CHANNEL_FORM_ID}`.
 */
export function CreateChannelFormFooter({
  form,
  submitLabel,
}: {
  form: CreateChannelFormState;
  submitLabel?: string;
}) {
  const { DurationIcon, durationLabel, isCreating, kindLabel } = form;

  return (
    <div className="flex w-full items-center justify-between gap-3">
      <Popover
        onOpenChange={form.setTypePopoverOpen}
        open={form.typePopoverOpen}
      >
        <PopoverTrigger asChild>
          <Button
            aria-label={`Channel duration: ${durationLabel}`}
            className="-ml-2.5 h-9 px-2.5 text-sm font-medium text-foreground hover:bg-muted/50"
            disabled={isCreating}
            type="button"
            variant="ghost"
          >
            <DurationIcon className="h-4 w-4" />
            {durationLabel}
            <ChevronDown className="h-4 w-4 text-muted-foreground/70" />
          </Button>
        </PopoverTrigger>
        <PopoverContent align="start" className="w-72 p-1">
          <div className="px-3 pb-1.5 pt-2 text-xs font-medium text-muted-foreground/70">
            Channel type
          </div>
          <fieldset className="space-y-1">
            <legend className="sr-only">Channel type</legend>
            <ChannelDurationOption
              ariaLabel="Ongoing channel"
              checked={!form.ephemeral}
              description="For projects, teams, and recurring conversations."
              icon={Hash}
              label="Ongoing"
              onSelect={() => {
                form.setEphemeral(false);
                form.setTypePopoverOpen(false);
              }}
            />
            <ChannelDurationOption
              ariaLabel="Ephemeral - auto-archives after 7 days of inactivity"
              checked={form.ephemeral}
              description="For quick discussions that archive automatically when inactive."
              icon={ClockFading}
              label="Temporary"
              onSelect={() => {
                form.setEphemeral(true);
                form.setTypePopoverOpen(false);
              }}
            />
          </fieldset>
        </PopoverContent>
      </Popover>
      <Button
        data-testid="create-channel-submit"
        disabled={!form.canSubmit}
        form={CREATE_CHANNEL_FORM_ID}
        type="submit"
      >
        {isCreating ? "Creating..." : (submitLabel ?? `Create ${kindLabel}`)}
      </Button>
    </div>
  );
}

function ChannelDurationOption({
  ariaLabel,
  checked,
  description,
  icon: Icon,
  label,
  onSelect,
}: {
  ariaLabel: string;
  checked: boolean;
  description: string;
  icon: typeof Hash;
  label: string;
  onSelect: () => void;
}) {
  return (
    <label
      className={cn(
        "relative flex min-h-16 cursor-pointer items-start gap-3 rounded-lg px-3 py-2.5 text-left text-muted-foreground/75 transition-colors duration-150 ease-out hover:bg-muted/50 hover:text-foreground has-[:focus-visible]:outline-hidden has-[:focus-visible]:ring-1 has-[:focus-visible]:ring-ring",
        checked && "text-foreground",
      )}
    >
      <input
        aria-label={ariaLabel}
        checked={checked}
        className="absolute inset-0 z-10 h-full w-full cursor-pointer opacity-0"
        name="create-channel-duration"
        onChange={onSelect}
        type="radio"
      />
      <span
        className={cn(
          "mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full border border-muted-foreground/40",
          checked && "border-foreground",
        )}
        aria-hidden="true"
      >
        <span
          className={cn(
            "h-1.5 w-1.5 rounded-full bg-foreground transition-opacity duration-150",
            checked ? "opacity-100" : "opacity-0",
          )}
        />
      </span>
      <span className="grid min-w-0 flex-1 grid-cols-[1rem_minmax(0,1fr)] gap-x-2 gap-y-1">
        <Icon className="h-4 w-4 shrink-0 text-current" />
        <span className="block text-sm font-medium leading-4 text-current">
          {label}
        </span>
        <span
          className={cn(
            "col-span-2 block text-xs leading-4 text-muted-foreground/70",
            checked && "text-muted-foreground/65",
          )}
        >
          {description}
        </span>
      </span>
    </label>
  );
}
