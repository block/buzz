import { ChevronDown } from "lucide-react";

import type { ChannelTemplate } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";

import {
  CHANNEL_FORM_FIELD_CONTROL_CLASS,
  CHANNEL_FORM_FIELD_SHELL_CLASS,
} from "@/features/channels/ui/channelFormStyles";
import { ChannelPermissionsSettings } from "@/features/channels/ui/ChannelPermissionsSettings";
import { ChannelTypeSettings } from "@/features/channels/ui/ChannelTypeSettings";
import type { CreateChannelFormState } from "@/features/sidebar/lib/useCreateChannelForm";

const CREATE_LABEL_OPTIONAL_CLASS =
  "ml-1 text-xs font-normal text-muted-foreground/50";
const NO_TEMPLATE_VALUE = "__no-template__";

export const CREATE_CHANNEL_FORM_ID = "create-channel-form";

/**
 * The body of the create-channel form (name, description, visibility,
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
  const selectedTemplate = form.templates.find(
    (template) => template.id === form.selectedTemplateId,
  );

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
            CHANNEL_FORM_FIELD_SHELL_CLASS,
          )}
        >
          <Input
            autoCapitalize="none"
            autoComplete="off"
            autoCorrect="off"
            className={cn(
              "h-8 px-0 py-0 leading-6",
              CHANNEL_FORM_FIELD_CONTROL_CLASS,
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
        <div className={CHANNEL_FORM_FIELD_SHELL_CLASS}>
          <Textarea
            className={cn(
              "min-h-20 resize-none px-3 py-3 leading-5",
              CHANNEL_FORM_FIELD_CONTROL_CLASS,
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

      <ChannelTypeSettings
        disabled={isCreating}
        onOpenChange={form.setTypePopoverOpen}
        onTemporaryChange={form.setEphemeral}
        onTtlSecondsChange={form.setTtlSeconds}
        open={form.typePopoverOpen}
        temporary={form.ephemeral}
        testIdPrefix="create-channel"
        ttlSeconds={form.ttlSeconds}
      />

      {form.templates.length > 0 ? (
        <div
          className={cn(
            "flex min-h-12 items-center justify-between gap-4 rounded-xl border border-input bg-background px-3 py-3",
            isCreating && "opacity-50",
          )}
        >
          <span className="text-sm font-medium text-foreground">
            Template
            <span className={CREATE_LABEL_OPTIONAL_CLASS}>Optional</span>
          </span>
          <DropdownMenu modal={false}>
            <DropdownMenuTrigger asChild>
              <Button
                aria-label={`Template: ${selectedTemplate?.name ?? "No template"}`}
                className="-mr-2.5 ml-auto h-9 min-w-0 max-w-[60%] justify-end px-2.5 text-right text-sm font-medium text-foreground hover:bg-muted/50"
                data-testid="create-channel-template"
                disabled={isCreating}
                id="create-channel-template"
                type="button"
                variant="ghost"
              >
                <span className="truncate text-right">
                  {selectedTemplate?.name ?? "No template"}
                </span>
                <ChevronDown className="size-4 shrink-0 text-muted-foreground/70" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent
              align="end"
              onCloseAutoFocus={(event) => event.preventDefault()}
              style={{
                minWidth: "var(--radix-dropdown-menu-trigger-width)",
              }}
            >
              <DropdownMenuRadioGroup
                onValueChange={(templateId) =>
                  form.handleTemplateChange(
                    templateId === NO_TEMPLATE_VALUE ? "" : templateId,
                  )
                }
                value={form.selectedTemplateId ?? NO_TEMPLATE_VALUE}
              >
                <DropdownMenuRadioItem value={NO_TEMPLATE_VALUE}>
                  No template
                </DropdownMenuRadioItem>
                {form.templates.map((template: ChannelTemplate) => (
                  <DropdownMenuRadioItem key={template.id} value={template.id}>
                    {template.name}
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      ) : null}

      <ChannelPermissionsSettings
        disabled={isCreating}
        onVisibilityChange={form.setVisibility}
        testIdPrefix="create-channel"
        visibility={form.visibility}
      />

      {form.errorMessage ? (
        <p className="text-sm text-destructive">{form.errorMessage}</p>
      ) : null}
    </div>
  );
}

/**
 * Footer for the create-channel form. The submit button is bound to the form
 * via `form={CREATE_CHANNEL_FORM_ID}`.
 */
export function CreateChannelFormFooter({
  form,
  submitLabel,
}: {
  form: CreateChannelFormState;
  submitLabel?: string;
}) {
  const { isCreating, kindLabel } = form;

  return (
    <div className="flex w-full items-center justify-end gap-3">
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
