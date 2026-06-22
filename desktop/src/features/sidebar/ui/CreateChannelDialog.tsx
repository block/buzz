import { ChevronDown, ClockFading, Hash, type LucideIcon } from "lucide-react";
import * as React from "react";

import { useChannelTemplatesQuery } from "@/features/channel-templates/hooks";
import type { ChannelTemplate, ChannelVisibility } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Switch } from "@/shared/ui/switch";
import { Textarea } from "@/shared/ui/textarea";

/** Default TTL for ephemeral channels: 7 days of inactivity. */
const EPHEMERAL_TTL_SECONDS = 604800;
const CREATE_FIELD_SHELL_CLASS =
  "rounded-xl border border-input bg-muted/40 transition-colors duration-150 ease-out hover:border-muted-foreground/40 focus-within:border-muted-foreground/50";
const CREATE_FIELD_CONTROL_CLASS =
  "border-0 bg-transparent text-muted-foreground/55 shadow-none outline-none ring-0 transition-colors duration-150 ease-out placeholder:text-muted-foreground/55 focus:bg-transparent focus:text-foreground focus:outline-hidden focus-visible:ring-0";
const CREATE_LABEL_OPTIONAL_CLASS =
  "ml-1 text-xs font-normal text-muted-foreground/50";

type ChannelKind = "stream" | "forum";

type CreateChannelDialogProps = {
  /** Which kind of channel to create, or null when closed. */
  channelKind: ChannelKind | null;
  isCreating: boolean;
  onOpenChange: (open: boolean) => void;
  onCreate: (input: {
    name: string;
    description?: string;
    visibility: ChannelVisibility;
    ttlSeconds?: number;
    templateId?: string;
  }) => Promise<void>;
};

// ---------------------------------------------------------------------------
// CreateChannelDialog
// ---------------------------------------------------------------------------

export function CreateChannelDialog({
  channelKind,
  isCreating,
  onOpenChange,
  onCreate,
}: CreateChannelDialogProps) {
  const open = channelKind !== null;
  const [name, setName] = React.useState("");
  const [description, setDescription] = React.useState("");
  const [visibility, setVisibility] = React.useState<ChannelVisibility>("open");
  const [ephemeral, setEphemeral] = React.useState(false);
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);
  const [selectedTemplateId, setSelectedTemplateId] = React.useState<
    string | null
  >(null);
  const [typePopoverOpen, setTypePopoverOpen] = React.useState(false);
  const nameInputRef = React.useRef<HTMLInputElement>(null);

  const templatesQuery = useChannelTemplatesQuery();
  const templates = templatesQuery.data ?? [];

  const kindLabel = channelKind === "forum" ? "forum" : "channel";
  const durationLabel = ephemeral ? "Temporary" : "Ongoing";
  const DurationIcon = ephemeral ? ClockFading : Hash;

  // Reset form state when dialog opens/closes or kind changes
  React.useEffect(() => {
    if (!open) return;

    setName("");
    setDescription("");
    setVisibility("open");
    setEphemeral(false);
    setErrorMessage(null);
    setSelectedTemplateId(null);
    setTypePopoverOpen(false);

    // Small delay to let dialog animation start before focusing
    const timerId = globalThis.setTimeout(() => {
      const activeElement = document.activeElement;
      if (
        activeElement instanceof HTMLElement &&
        activeElement.closest("#create-channel-form")
      ) {
        return;
      }
      nameInputRef.current?.focus();
    }, 50);
    return () => globalThis.clearTimeout(timerId);
  }, [open]);

  function handleTemplateChange(templateId: string) {
    if (!templateId) {
      setSelectedTemplateId(null);
      setDescription("");
      setVisibility("open");
      setErrorMessage(null);
      return;
    }

    const template = templates.find(
      (t: ChannelTemplate) => t.id === templateId,
    );
    if (!template) return;

    setSelectedTemplateId(templateId);

    // Pre-fill fields from template (always overwrite to avoid stale values)
    setDescription(template.description ?? "");
    setVisibility(template.visibility);

    // If the template's channel type differs from current dialog kind,
    // we still apply the visibility but don't change the kind
    // (kind is determined by how the dialog was opened)
    setErrorMessage(null);
  }

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();

    const trimmedName = name.trim();
    if (!trimmedName) return;

    setErrorMessage(null);

    try {
      await onCreate({
        name: trimmedName,
        description: description.trim() || undefined,
        visibility,
        ttlSeconds: ephemeral ? EPHEMERAL_TTL_SECONDS : undefined,
        templateId: selectedTemplateId ?? undefined,
      });

      onOpenChange(false);
    } catch (error) {
      setErrorMessage(
        error instanceof Error
          ? error.message
          : `Failed to create ${kindLabel}.`,
      );
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen && isCreating) return;
        onOpenChange(nextOpen);
      }}
    >
      <ChooserDialogContent
        className="max-w-lg"
        contentClassName="pt-3"
        data-testid="create-channel-dialog"
        footerClassName="border-t-0 pt-0"
        headerClassName="pb-2"
        title={`Create a new ${kindLabel}`}
        description={
          channelKind === "forum"
            ? "Forums organize threaded discussions around a topic."
            : "Channels are real-time streams for team conversation."
        }
        footer={
          <div className="flex w-full items-center justify-between gap-3">
            <Popover onOpenChange={setTypePopoverOpen} open={typePopoverOpen}>
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
                    checked={!ephemeral}
                    description="For projects, teams, and recurring conversations."
                    icon={Hash}
                    label="Ongoing"
                    onSelect={() => {
                      setEphemeral(false);
                      setTypePopoverOpen(false);
                    }}
                  />
                  <ChannelDurationOption
                    ariaLabel="Ephemeral - auto-archives after 7 days of inactivity"
                    checked={ephemeral}
                    description="For quick discussions that archive automatically when inactive."
                    icon={ClockFading}
                    label="Temporary"
                    onSelect={() => {
                      setEphemeral(true);
                      setTypePopoverOpen(false);
                    }}
                  />
                </fieldset>
              </PopoverContent>
            </Popover>
            <Button
              data-testid="create-channel-submit"
              disabled={isCreating || name.trim().length === 0}
              form="create-channel-form"
              type="submit"
            >
              {isCreating ? "Creating..." : `Create ${kindLabel}`}
            </Button>
          </div>
        }
      >
        <form
          className="space-y-5"
          id="create-channel-form"
          onSubmit={(event) => {
            void handleSubmit(event);
          }}
        >
          {/* Name */}
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
                onChange={(event) => {
                  setName(event.target.value);
                  setErrorMessage(null);
                }}
                placeholder={
                  channelKind === "forum"
                    ? "design-discussions"
                    : "release-notes"
                }
                ref={nameInputRef}
                spellCheck={false}
                value={name}
              />
            </div>
          </div>

          {/* Description */}
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
                onChange={(event) => {
                  setDescription(event.target.value);
                  setErrorMessage(null);
                }}
                placeholder={`What this ${kindLabel} is for`}
                rows={2}
                value={description}
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
              checked={visibility === "private"}
              className="shrink-0 shadow-none [&>span]:shadow-none"
              data-testid="create-channel-private-toggle"
              disabled={isCreating}
              id="create-channel-private"
              onCheckedChange={(checked) =>
                setVisibility(checked ? "private" : "open")
              }
            />
          </div>

          {/* Template Selector */}
          {templates.length > 0 ? (
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
                onChange={(event) => handleTemplateChange(event.target.value)}
                value={selectedTemplateId ?? ""}
              >
                <option value="">No template</option>
                {templates.map((template: ChannelTemplate) => (
                  <option key={template.id} value={template.id}>
                    {template.name}
                  </option>
                ))}
              </select>
            </div>
          ) : null}

          {/* Error */}
          {errorMessage ? (
            <p className="text-sm text-destructive">{errorMessage}</p>
          ) : null}
        </form>
      </ChooserDialogContent>
    </Dialog>
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
  icon: LucideIcon;
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
