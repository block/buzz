import { ClockFading, Hash } from "lucide-react";
import * as React from "react";

import { useChannelTemplatesQuery } from "@/features/channel-templates/hooks";
import type { ChannelTemplate, ChannelVisibility } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import { Textarea } from "@/shared/ui/textarea";

/** Default TTL for ephemeral channels: 1 day of inactivity. */
const EPHEMERAL_TTL_SECONDS = 86400;
const CREATE_FIELD_SHELL_CLASS =
  "rounded-xl border border-input bg-background shadow-xs transition-colors duration-150 ease-out hover:border-muted-foreground/40 hover:bg-muted/70 focus-within:border-muted-foreground/50 focus-within:bg-muted/70";
const CREATE_FIELD_CONTROL_CLASS =
  "border-0 bg-transparent shadow-none outline-none ring-0 placeholder:text-muted-foreground/45 focus:bg-transparent focus:outline-hidden focus-visible:ring-0";

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
  const nameInputRef = React.useRef<HTMLInputElement>(null);

  const templatesQuery = useChannelTemplatesQuery();
  const templates = templatesQuery.data ?? [];

  const kindLabel = channelKind === "forum" ? "forum" : "channel";

  // Reset form state when dialog opens/closes or kind changes
  React.useEffect(() => {
    if (!open) return;

    setName("");
    setDescription("");
    setVisibility("open");
    setEphemeral(false);
    setErrorMessage(null);
    setSelectedTemplateId(null);

    // Small delay to let dialog animation start before focusing
    const timerId = globalThis.setTimeout(() => {
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
          <div className="flex w-full items-center justify-end gap-2">
            <Button
              disabled={isCreating}
              onClick={() => onOpenChange(false)}
              type="button"
              variant="ghost"
            >
              Cancel
            </Button>
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
                "flex h-9 items-center px-3",
                CREATE_FIELD_SHELL_CLASS,
              )}
            >
              <Input
                autoCapitalize="none"
                autoComplete="off"
                autoCorrect="off"
                className={cn("h-full px-0 py-0", CREATE_FIELD_CONTROL_CLASS)}
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
              Description{" "}
              <span className="font-normal text-muted-foreground">
                (optional)
              </span>
            </label>
            <div className={CREATE_FIELD_SHELL_CLASS}>
              <Textarea
                className={cn(
                  "min-h-16 resize-none px-3 py-2",
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

          {/* Type */}
          <div className="space-y-2">
            <p className="text-sm font-medium text-foreground">Type</p>
            <Tabs
              className="w-full"
              data-testid="create-channel-ephemeral"
              onValueChange={(value) => setEphemeral(value === "temporary")}
              value={ephemeral ? "temporary" : "ongoing"}
            >
              <TabsList className="relative grid h-auto w-full grid-cols-2 items-stretch rounded-xl bg-muted/45 p-1 text-muted-foreground">
                <span
                  aria-hidden="true"
                  className="pointer-events-none absolute bottom-1 left-1 top-1 z-0 w-[calc(50%-0.25rem)] rounded-lg bg-background/95 shadow-xs transition-transform duration-[180ms] ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none"
                  data-testid="create-channel-type-indicator"
                  style={{
                    transform: ephemeral
                      ? "translate3d(100%, 0, 0)"
                      : "translate3d(0, 0, 0)",
                  }}
                />
                <TabsTrigger
                  className="group/type-tab relative z-10 h-full min-h-24 flex-col items-start justify-start gap-1.5 whitespace-normal rounded-lg bg-transparent px-3 py-2.5 text-left text-muted-foreground/55 opacity-70 shadow-none transition-[color,opacity] duration-150 ease-[cubic-bezier(0.23,1,0.32,1)] hover:text-muted-foreground hover:opacity-85 data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:opacity-100 data-[state=active]:shadow-none motion-reduce:transition-none"
                  disabled={isCreating}
                  value="ongoing"
                >
                  <Hash className="mt-0.5 h-4 w-4 shrink-0 text-current" />
                  <span className="min-w-0 space-y-0.5">
                    <span className="block text-sm font-medium">Ongoing</span>
                    <span className="block text-xs leading-4 text-muted-foreground/45 group-data-[state=active]/type-tab:text-muted-foreground/65">
                      For projects, teams, and recurring conversations.
                    </span>
                  </span>
                </TabsTrigger>
                <TabsTrigger
                  aria-label="Ephemeral — auto-archives after 1 day of inactivity"
                  className="group/type-tab relative z-10 h-full min-h-24 flex-col items-start justify-start gap-1.5 whitespace-normal rounded-lg bg-transparent px-3 py-2.5 text-left text-muted-foreground/55 opacity-70 shadow-none transition-[color,opacity] duration-150 ease-[cubic-bezier(0.23,1,0.32,1)] hover:text-muted-foreground hover:opacity-85 data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:opacity-100 data-[state=active]:shadow-none motion-reduce:transition-none"
                  disabled={isCreating}
                  value="temporary"
                >
                  <ClockFading className="mt-0.5 h-4 w-4 shrink-0 text-current" />
                  <span className="min-w-0 space-y-0.5">
                    <span className="block text-sm font-medium">Temporary</span>
                    <span className="block text-xs leading-4 text-muted-foreground/45 group-data-[state=active]/type-tab:text-muted-foreground/65">
                      For quick discussions that archive automatically when
                      inactive.
                    </span>
                  </span>
                </TabsTrigger>
              </TabsList>
            </Tabs>
          </div>

          {/* Permissions */}
          <fieldset
            className="space-y-2"
            data-testid="create-channel-visibility"
          >
            <legend className="text-sm font-medium text-foreground">
              Permissions
            </legend>
            <div className="space-y-1">
              <PermissionOption
                description={`Anyone can see and join this ${kindLabel}.`}
                disabled={isCreating}
                name="create-channel-visibility"
                onChange={() => setVisibility("open")}
                selected={visibility === "open"}
                title="Open"
                value="open"
              />
              <PermissionOption
                description={`Only members can invite people to this ${kindLabel}.`}
                disabled={isCreating}
                name="create-channel-visibility"
                onChange={() => setVisibility("private")}
                selected={visibility === "private"}
                title="Private"
                value="private"
              />
            </div>
          </fieldset>

          {/* Template Selector */}
          {templates.length > 0 ? (
            <div className="space-y-1.5">
              <label
                className="text-sm font-medium text-foreground"
                htmlFor="create-channel-template"
              >
                Template{" "}
                <span className="font-normal text-muted-foreground">
                  (optional)
                </span>
              </label>
              <select
                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
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

function PermissionOption({
  description,
  disabled,
  name,
  onChange,
  selected,
  title,
  value,
}: {
  description: string;
  disabled: boolean;
  name: string;
  onChange: () => void;
  selected: boolean;
  title: string;
  value: ChannelVisibility;
}) {
  return (
    <label
      className={cn(
        "group flex cursor-pointer items-start gap-3 rounded-lg px-1 py-1.5 transition-none",
        "has-[:focus-visible]:outline-hidden has-[:focus-visible]:ring-1 has-[:focus-visible]:ring-ring",
        selected
          ? "text-foreground"
          : "text-muted-foreground hover:text-foreground",
        disabled && "pointer-events-none opacity-50",
      )}
    >
      <input
        checked={selected}
        className="sr-only"
        disabled={disabled}
        name={name}
        onChange={onChange}
        type="radio"
        value={value}
      />
      <span
        className={cn(
          "mt-1 flex h-4 w-4 shrink-0 items-center justify-center rounded-full border transition-none",
          selected
            ? "border-foreground"
            : "border-muted-foreground/45 group-hover:border-muted-foreground",
        )}
        aria-hidden="true"
      >
        <span
          className={cn(
            "h-1.5 w-1.5 rounded-full bg-foreground transition-[opacity,transform] duration-150 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none",
            selected ? "scale-100 opacity-100" : "scale-50 opacity-0",
          )}
        />
      </span>
      <span className="min-w-0 space-y-0.5">
        <span className="block text-sm font-medium text-current">{title}</span>
        <span className="block text-xs leading-4 text-muted-foreground/65">
          {description}
        </span>
      </span>
    </label>
  );
}
