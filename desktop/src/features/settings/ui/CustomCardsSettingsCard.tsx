import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ExternalLink, KeyRound, Loader2, Trash2 } from "lucide-react";
import { toast } from "sonner";

import {
  cardMintDedicatedKeyStatus,
  cardMintDeleteOpenaiKey,
  cardMintSaveOpenaiKey,
} from "@/shared/api/tauriPersonas";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

const OPENAI_KEYS_URL = "https://platform.openai.com/api-keys";
const DEDICATED_KEY_QUERY = ["cardMintDedicatedKeyStatus"] as const;

export function CustomCardsSettingsCard() {
  const [editing, setEditing] = React.useState(false);
  const [keyDraft, setKeyDraft] = React.useState("");
  const queryClient = useQueryClient();
  const statusQuery = useQuery({
    queryKey: DEDICATED_KEY_QUERY,
    queryFn: cardMintDedicatedKeyStatus,
  });

  const finishCredentialChange = React.useCallback(
    (configured: boolean) => {
      queryClient.setQueryData(DEDICATED_KEY_QUERY, configured);
      void queryClient.invalidateQueries({ queryKey: ["cardMintKeyStatus"] });
      setKeyDraft("");
      setEditing(false);
    },
    [queryClient],
  );

  const saveMutation = useMutation({
    mutationFn: cardMintSaveOpenaiKey,
    onSuccess: () => {
      finishCredentialChange(true);
      toast.success("OpenAI key saved for custom cards only.");
    },
    onError: (error) =>
      toast.error(
        error instanceof Error
          ? error.message
          : "Could not save the custom-card API key.",
      ),
  });

  const deleteMutation = useMutation({
    mutationFn: cardMintDeleteOpenaiKey,
    onSuccess: () => {
      finishCredentialChange(false);
      toast.success("Custom-card API key removed.");
    },
    onError: (error) =>
      toast.error(
        error instanceof Error
          ? error.message
          : "Could not remove the custom-card API key.",
      ),
  });

  const pending = saveMutation.isPending || deleteMutation.isPending;
  const configured = statusQuery.data === true;

  return (
    <SettingsOptionGroup
      data-testid="settings-custom-cards"
      description="Use a separate OpenAI API key for custom-card creation without changing the provider, model, or environment inherited by agents."
      title="Custom cards"
    >
      <SettingsOptionRow>
        <div className="min-w-0">
          <div className="flex items-center gap-2 text-sm font-medium">
            <KeyRound className="h-4 w-4 text-muted-foreground" />
            OpenAI API key
          </div>
          <p
            className="mt-1 text-sm font-normal text-muted-foreground/70"
            data-settings-subcopy
          >
            {statusQuery.isError
              ? "The saved-key status could not be loaded."
              : configured
                ? "Stored securely and used only for custom cards."
                : "No dedicated custom-card key is saved."}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {statusQuery.isPending ? (
            <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
          ) : (
            <>
              {configured ? (
                <Button
                  aria-label="Remove custom-card API key"
                  data-testid="custom-card-key-remove"
                  disabled={pending}
                  onClick={() => deleteMutation.mutate()}
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  <Trash2 className="h-4 w-4" />
                  Remove
                </Button>
              ) : null}
              <Button
                data-testid="custom-card-key-edit"
                disabled={pending}
                onClick={() => setEditing(true)}
                size="sm"
                type="button"
                variant="outline"
              >
                {configured ? "Update" : "Add key"}
              </Button>
            </>
          )}
        </div>
      </SettingsOptionRow>

      {editing ? (
        <div
          className="space-y-3 px-4 py-4"
          data-testid="custom-card-key-editor"
        >
          <Input
            autoFocus
            data-testid="custom-card-key-input"
            disabled={pending}
            onChange={(event) => setKeyDraft(event.target.value)}
            placeholder="sk-…"
            type="password"
            value={keyDraft}
          />
          <div className="flex items-center justify-between gap-3">
            <Button
              className="h-auto px-0 text-xs"
              onClick={() =>
                void openUrl(OPENAI_KEYS_URL).catch(() => {
                  toast.error("Failed to open link");
                })
              }
              size="sm"
              type="button"
              variant="link"
            >
              <ExternalLink className="h-3 w-3" />
              Get a key at platform.openai.com
            </Button>
            <div className="flex items-center gap-2">
              <Button
                data-testid="custom-card-key-cancel"
                disabled={pending}
                onClick={() => {
                  setKeyDraft("");
                  setEditing(false);
                }}
                size="sm"
                type="button"
                variant="ghost"
              >
                Cancel
              </Button>
              <Button
                data-testid="custom-card-key-save"
                disabled={pending || keyDraft.trim().length === 0}
                onClick={() => saveMutation.mutate(keyDraft.trim())}
                size="sm"
                type="button"
              >
                {saveMutation.isPending ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <KeyRound className="h-4 w-4" />
                )}
                Save key
              </Button>
            </div>
          </div>
        </div>
      ) : null}
    </SettingsOptionGroup>
  );
}
