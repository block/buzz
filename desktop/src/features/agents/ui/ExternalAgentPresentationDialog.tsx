import * as React from "react";

import {
  saveExternalAgentPresentation,
  type ExternalAgentPresentation,
} from "@/features/agents/externalAgentPresentation";
import { ProfileAvatarEditor } from "@/features/profile/ui/ProfileAvatarEditor";
import type { RelayAgent } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";

export function ExternalAgentPresentationDialog({
  agent,
  onOpenChange,
  open,
  scope,
}: {
  agent: RelayAgent | null;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  scope: string | null;
}) {
  const [displayName, setDisplayName] = React.useState("");
  const [avatarUrl, setAvatarUrl] = React.useState("");
  const [isUploading, setIsUploading] = React.useState(false);

  React.useEffect(() => {
    if (!open || !agent) return;
    setDisplayName(agent.name);
    setAvatarUrl(agent.avatarUrl ?? "");
    setIsUploading(false);
  }, [agent, open]);

  if (!agent) return null;

  const save = (presentation: ExternalAgentPresentation | null) => {
    if (!scope) return;
    saveExternalAgentPresentation(scope, agent.pubkey, presentation);
    onOpenChange(false);
  };

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        aria-describedby="external-agent-presentation-description"
        className="max-h-[calc(100vh-2rem)] overflow-y-auto"
        data-testid="external-agent-presentation-dialog"
      >
        <DialogHeader>
          <DialogTitle>Customize external agent</DialogTitle>
          <DialogDescription id="external-agent-presentation-description">
            This changes how the agent appears in your Buzz Desktop. It never
            edits the external runtime, Soul, prompts, memory, or provider
            files.
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-5">
          <label
            className="grid gap-2 text-sm font-medium"
            htmlFor="external-agent-display-name"
          >
            Display name
            <Input
              autoFocus
              data-testid="external-agent-display-name"
              id="external-agent-display-name"
              maxLength={80}
              onChange={(event) => setDisplayName(event.target.value)}
              placeholder={agent.name}
              value={displayName}
            />
          </label>

          <div className="grid gap-2">
            <p className="text-sm font-medium">Icon / avatar</p>
            <ProfileAvatarEditor
              avatarUrl={avatarUrl}
              disabled={isUploading}
              onUploadedAvatarChange={(url) => {
                if (url) setAvatarUrl(url);
              }}
              onUploadingChange={setIsUploading}
              onUrlChange={setAvatarUrl}
              previewName={displayName.trim() || agent.name}
              testIdPrefix="external-agent-avatar"
            />
          </div>
        </div>

        <DialogFooter>
          <Button
            disabled={isUploading}
            onClick={() => save(null)}
            type="button"
            variant="ghost"
          >
            Reset
          </Button>
          <Button
            disabled={isUploading || !displayName.trim()}
            onClick={() =>
              save({
                displayName: displayName.trim(),
                avatarUrl: avatarUrl.trim() || null,
              })
            }
            type="button"
          >
            Save appearance
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
