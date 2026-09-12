import { invoke } from "@tauri-apps/api/core";
import { LoaderCircle } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import {
  SettingsOptionGroup,
  SettingsOptionRow,
} from "@/features/settings/ui/SettingsOptionGroup";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Textarea } from "@/shared/ui/textarea";

import {
  DEFAULT_MESSAGE,
  WelcomeComposer,
  type WelcomeMessage,
  WelcomePreview,
} from "./WelcomeChannelSettingsCard";

type ComposerMode = "edit" | "preview";
type CreationMode = "assist" | "manual";

export function WelcomeChannelSettings() {
  const [builderOpen, setBuilderOpen] = React.useState(false);
  const [creationMode, setCreationMode] =
    React.useState<CreationMode>("manual");
  const [mode, setMode] = React.useState<ComposerMode>("edit");
  const [message, setMessage] = React.useState(DEFAULT_MESSAGE);
  const [savedMessage, setSavedMessage] = React.useState<WelcomeMessage | null>(
    null,
  );
  const [assistantPrompt, setAssistantPrompt] = React.useState("");
  const [isGenerating, setIsGenerating] = React.useState(false);
  const [imageUploadsInFlight, setImageUploadsInFlight] = React.useState(0);
  const isDirty =
    !savedMessage || JSON.stringify(message) !== JSON.stringify(savedMessage);

  function openBuilder(nextMode: CreationMode) {
    setCreationMode(nextMode);
    setMode("edit");
    setBuilderOpen(true);
  }

  async function createAssistedDraft() {
    if (!assistantPrompt.trim() || isGenerating) return;
    setIsGenerating(true);
    try {
      const draft = await invoke<WelcomeMessage>("generate_welcome_message", {
        request: assistantPrompt.trim(),
      });
      setMessage(draft);
      setCreationMode("manual");
      setMode("edit");
    } catch (error) {
      const message =
        typeof error === "string" && error.startsWith("Writing help")
          ? error
          : "Writing help couldn’t create a message. Try again.";
      toast.error(message);
    } finally {
      setIsGenerating(false);
    }
  }

  function saveMessage() {
    setSavedMessage(message);
    setBuilderOpen(false);
    toast.success("Welcome message saved");
  }

  return (
    <section className="mt-12" data-testid="welcome-channel-settings">
      <SettingsOptionGroup>
        <SettingsOptionRow data-testid="welcome-channel-row">
          <div className="min-w-0 flex-1">
            <p className="text-sm font-medium">Custom welcome message</p>
            <p
              className="text-xs text-muted-foreground/70"
              data-settings-subcopy
            >
              Create a message new members will see in their own Welcome
              channel.
            </p>
          </div>
          <Button
            onClick={() => openBuilder("manual")}
            size="sm"
            type="button"
            variant="outline"
          >
            {savedMessage ? "Edit" : "Create"}
          </Button>
        </SettingsOptionRow>
      </SettingsOptionGroup>

      <Dialog onOpenChange={setBuilderOpen} open={builderOpen}>
        <DialogContent
          className="flex flex-col sm:min-h-[36rem] sm:max-w-4xl"
          data-testid="welcome-message-builder"
        >
          <DialogHeader>
            <DialogTitle>
              {creationMode === "assist"
                ? "Get writing help"
                : "Custom welcome message"}
            </DialogTitle>
            <DialogDescription>
              {creationMode === "assist"
                ? "Describe what new members should know, and generate a draft you can edit."
                : "Welcome new members and share helpful information, channels, and community practices. Right-click to add names, channels, images, or links."}
            </DialogDescription>
          </DialogHeader>

          {creationMode === "assist" ? (
            <div className="flex flex-1 flex-col gap-5 py-2">
              <div className="flex min-h-0 flex-1 flex-col gap-2">
                <Textarea
                  aria-label="What should your welcome message do?"
                  autoFocus
                  className="min-h-0 flex-1 resize-none rounded-2xl border-border/50 bg-background/80 px-4 py-3 text-base shadow-none placeholder:text-muted-foreground/65"
                  onChange={(event) => setAssistantPrompt(event.target.value)}
                  placeholder="Welcome new members by name. Ask them to introduce themselves in #introductions, then share our community guide and team photo."
                  value={assistantPrompt}
                />
                <p className="text-xs text-muted-foreground">
                  Describe the tone and include any links, images, or channels
                  you want to share.
                </p>
              </div>

              <DialogFooter className="mt-auto">
                <Button
                  onClick={() => setCreationMode("manual")}
                  type="button"
                  variant="ghost"
                >
                  Back to editor
                </Button>
                <Button
                  aria-busy={isGenerating}
                  disabled={!assistantPrompt.trim() || isGenerating}
                  onClick={createAssistedDraft}
                  type="button"
                >
                  {isGenerating ? (
                    <LoaderCircle
                      aria-hidden="true"
                      className="animate-spin"
                      data-testid="welcome-generation-spinner"
                    />
                  ) : null}
                  Create draft
                </Button>
              </DialogFooter>
            </div>
          ) : (
            <div className="flex flex-1 flex-col gap-5 py-2">
              <fieldset className="inline-flex self-start rounded-lg bg-muted p-1">
                <legend className="sr-only">Welcome view</legend>
                {(["edit", "preview"] as const).map((nextMode) => (
                  <button
                    aria-pressed={mode === nextMode}
                    className={cn(
                      "rounded-md px-4 py-1.5 text-sm font-medium capitalize text-muted-foreground transition-colors",
                      mode === nextMode &&
                        "bg-background text-foreground shadow-xs",
                    )}
                    key={nextMode}
                    onClick={() => setMode(nextMode)}
                    type="button"
                  >
                    {nextMode}
                  </button>
                ))}
              </fieldset>

              {mode === "edit" ? (
                <div className="min-h-64 flex-1 rounded-2xl border border-border/50 bg-background/80 px-4 py-3 shadow-none transition-colors focus-within:ring-1 focus-within:ring-ring">
                  <WelcomeComposer
                    message={message}
                    onChange={setMessage}
                    onUploadCountChange={setImageUploadsInFlight}
                  />
                </div>
              ) : (
                <WelcomePreview message={message} />
              )}

              <DialogFooter className="mt-auto">
                <Button
                  onClick={() => setCreationMode("assist")}
                  type="button"
                  variant="ghost"
                >
                  Get writing help
                </Button>
                <Button
                  disabled={!isDirty || imageUploadsInFlight > 0}
                  onClick={saveMessage}
                  type="button"
                >
                  Save
                </Button>
              </DialogFooter>
            </div>
          )}
        </DialogContent>
      </Dialog>
    </section>
  );
}
