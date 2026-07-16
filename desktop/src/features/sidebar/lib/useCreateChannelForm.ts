import { ClockFading, Hash, type LucideIcon } from "lucide-react";
import * as React from "react";

import { useChannelTemplatesQuery } from "@/features/channel-templates/hooks";
import { DEFAULT_EPHEMERAL_TTL_SECONDS } from "@/features/channels/lib/ephemeralChannel";
import type { ChannelTemplate, ChannelVisibility } from "@/shared/api/types";

export type CreateChannelKind = "stream" | "forum";

export type CreateChannelInput = {
  name: string;
  description?: string;
  visibility: ChannelVisibility;
  ttlSeconds?: number;
  templateId?: string;
};

type UseCreateChannelFormOptions = {
  channelKind: CreateChannelKind;
  /**
   * When this flips to `true` the form resets its fields (and applies
   * `initialName`). Pass the dialog/mode's open state.
   */
  active: boolean;
  initialName?: string;
  isCreating: boolean;
  onCreate: (input: CreateChannelInput) => Promise<void>;
  onCreated?: () => void;
  autoFocusName?: boolean;
};

export type CreateChannelFormState = {
  channelKind: CreateChannelKind;
  kindLabel: string;
  name: string;
  setName: (value: string) => void;
  description: string;
  setDescription: (value: string) => void;
  visibility: ChannelVisibility;
  setVisibility: (value: ChannelVisibility) => void;
  ephemeral: boolean;
  setEphemeral: (value: boolean) => void;
  durationLabel: string;
  DurationIcon: LucideIcon;
  typePopoverOpen: boolean;
  setTypePopoverOpen: (open: boolean) => void;
  errorMessage: string | null;
  selectedTemplateId: string | null;
  handleTemplateChange: (templateId: string) => void;
  templates: ChannelTemplate[];
  nameInputRef: React.RefObject<HTMLInputElement | null>;
  isCreating: boolean;
  canSubmit: boolean;
  handleSubmit: (event: React.FormEvent<HTMLFormElement>) => void;
};

/**
 * Shared state + submit logic for the create-channel form. Powers both the
 * standalone `CreateChannelDialog` and the create mode of the unified
 * "Add channel" browser dialog, so the two stay behaviorally identical.
 */
export function useCreateChannelForm({
  channelKind,
  active,
  initialName,
  isCreating,
  onCreate,
  onCreated,
  autoFocusName = true,
}: UseCreateChannelFormOptions): CreateChannelFormState {
  const [name, setName] = React.useState(initialName ?? "");
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

  React.useEffect(() => {
    if (!active) return;

    setName(initialName ?? "");
    setDescription("");
    setVisibility("open");
    setEphemeral(false);
    setErrorMessage(null);
    setSelectedTemplateId(null);
    setTypePopoverOpen(false);

    if (!autoFocusName) return;

    // Small delay to let the dialog animation start before focusing.
    const timerId = globalThis.setTimeout(() => {
      const activeElement = document.activeElement;
      if (
        activeElement instanceof HTMLElement &&
        activeElement.closest("#create-channel-form")
      ) {
        return;
      }
      const input = nameInputRef.current;
      if (!input) return;
      input.focus();
      // Place the caret at the end of any prefilled name.
      const end = input.value.length;
      input.setSelectionRange(end, end);
    }, 50);
    return () => globalThis.clearTimeout(timerId);
  }, [active, autoFocusName, initialName]);

  const handleTemplateChange = React.useCallback(
    (templateId: string) => {
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
      setDescription(template.description ?? "");
      setVisibility(template.visibility);
      setErrorMessage(null);
    },
    [templates],
  );

  const handleSubmit = React.useCallback(
    (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();

      const trimmedName = name.trim();
      if (!trimmedName) return;

      setErrorMessage(null);

      void (async () => {
        try {
          await onCreate({
            name: trimmedName,
            description: description.trim() || undefined,
            visibility,
            ttlSeconds: ephemeral ? DEFAULT_EPHEMERAL_TTL_SECONDS : undefined,
            templateId: selectedTemplateId ?? undefined,
          });
          onCreated?.();
        } catch (error) {
          setErrorMessage(
            error instanceof Error
              ? error.message
              : `Failed to create ${kindLabel}.`,
          );
        }
      })();
    },
    [
      description,
      ephemeral,
      kindLabel,
      name,
      onCreate,
      onCreated,
      selectedTemplateId,
      visibility,
    ],
  );

  return {
    channelKind,
    kindLabel,
    name,
    setName: (value: string) => {
      setName(value);
      setErrorMessage(null);
    },
    description,
    setDescription: (value: string) => {
      setDescription(value);
      setErrorMessage(null);
    },
    visibility,
    setVisibility,
    ephemeral,
    setEphemeral,
    durationLabel,
    DurationIcon,
    typePopoverOpen,
    setTypePopoverOpen,
    errorMessage,
    selectedTemplateId,
    handleTemplateChange,
    templates,
    nameInputRef,
    isCreating,
    canSubmit: name.trim().length > 0 && !isCreating,
    handleSubmit,
  };
}
