import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ImagePlus, KeyRound, Loader2 } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { evictUsersBatchEntries } from "@/features/profile/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import {
  getExternalAgentIdentityStatus,
  linkExternalAgentIdentity,
  updateExternalAgentProfile,
} from "@/shared/api/tauriExternalAgentIdentity";
import { pickAndUploadImage } from "@/shared/api/tauriMedia";
import type { Profile } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";

const identityStatusQueryKey = (pubkey: string) =>
  ["external-agent-identity", pubkey.toLowerCase()] as const;

export function useExternalAgentIdentityStatus(pubkey: string) {
  return useQuery({
    queryKey: identityStatusQueryKey(pubkey),
    queryFn: () => getExternalAgentIdentityStatus(pubkey),
    staleTime: 30_000,
  });
}

export function ExternalAgentIdentityDialog({
  name,
  onOpenChange,
  open,
  profile,
  pubkey,
}: {
  name: string;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  profile: Profile | undefined;
  pubkey: string;
}) {
  const queryClient = useQueryClient();
  const statusQuery = useExternalAgentIdentityStatus(pubkey);
  const [nsec, setNsec] = React.useState("");
  const [displayName, setDisplayName] = React.useState("");
  const [avatarUrl, setAvatarUrl] = React.useState("");
  const [about, setAbout] = React.useState("");
  const [pendingAction, setPendingAction] = React.useState<
    "link" | "save" | "image" | null
  >(null);

  React.useEffect(() => {
    if (!open) {
      setNsec("");
      return;
    }
    setDisplayName(profile?.displayName?.trim() || name);
    setAvatarUrl(profile?.avatarUrl?.trim() || "");
    setAbout(profile?.about || "");
  }, [name, open, profile]);

  const linked = statusQuery.data?.linked === true;
  const busy = pendingAction !== null;

  async function handleLink() {
    setPendingAction("link");
    try {
      await linkExternalAgentIdentity(pubkey, nsec);
      setNsec("");
      await queryClient.invalidateQueries({
        queryKey: identityStatusQueryKey(pubkey),
      });
      toast.success(`${name} is linked securely.`);
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Couldn’t link this agent.",
      );
    } finally {
      setPendingAction(null);
    }
  }

  async function handleChooseImage() {
    setPendingAction("image");
    try {
      const image = await pickAndUploadImage();
      if (image) setAvatarUrl(image.url);
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Couldn’t upload the image.",
      );
    } finally {
      setPendingAction(null);
    }
  }

  async function handleSave() {
    setPendingAction("save");
    try {
      await updateExternalAgentProfile(pubkey, {
        displayName: displayName.trim(),
        avatarUrl: avatarUrl.trim(),
        about: about.trim(),
      });
      evictUsersBatchEntries(queryClient, [pubkey]);
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: ["user-profile", pubkey.toLowerCase()],
        }),
        queryClient.invalidateQueries({
          predicate: (query) =>
            query.queryKey[0] === "users-batch" &&
            query.queryKey.includes(pubkey.toLowerCase()),
        }),
      ]);
      toast.success(`Updated ${displayName.trim() || name}.`);
      onOpenChange(false);
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Couldn’t update this agent.",
      );
    } finally {
      setPendingAction(null);
    }
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-lg overflow-hidden p-0">
        <DialogHeader className="border-b border-border/60 px-6 py-5 pr-14">
          <DialogTitle>{linked ? `Edit ${name}` : `Link ${name}`}</DialogTitle>
          <DialogDescription>
            {linked
              ? "Edit the public Buzz profile. The Hermes model, memory, and runtime stay managed by Hermes."
              : "Link the existing Hermes identity so Buzz can edit its public name, photo, and description."}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 px-6 py-5">
          {statusQuery.isLoading ? (
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" />
              Checking secure storage…
            </div>
          ) : linked ? (
            <>
              <div className="flex items-center gap-4">
                <ProfileAvatar
                  avatarUrl={avatarUrl || null}
                  className="h-16 w-16 shrink-0"
                  label={displayName || name}
                />
                <div className="min-w-0 flex-1 space-y-2">
                  <label
                    className="block text-sm font-medium"
                    htmlFor="external-agent-name"
                  >
                    Name
                  </label>
                  <Input
                    id="external-agent-name"
                    maxLength={80}
                    onChange={(event) => setDisplayName(event.target.value)}
                    value={displayName}
                  />
                </div>
              </div>

              <div className="space-y-2">
                <div className="flex items-center justify-between gap-3">
                  <label
                    className="text-sm font-medium"
                    htmlFor="external-agent-avatar"
                  >
                    Photo
                  </label>
                  <Button
                    disabled={busy}
                    onClick={() => void handleChooseImage()}
                    size="sm"
                    type="button"
                    variant="outline"
                  >
                    {pendingAction === "image" ? (
                      <Loader2 className="h-4 w-4 animate-spin" />
                    ) : (
                      <ImagePlus className="h-4 w-4" />
                    )}
                    Choose image
                  </Button>
                </div>
                <Input
                  id="external-agent-avatar"
                  onChange={(event) => setAvatarUrl(event.target.value)}
                  placeholder="https://…"
                  value={avatarUrl}
                />
              </div>

              <div className="space-y-2">
                <label
                  className="block text-sm font-medium"
                  htmlFor="external-agent-about"
                >
                  Description
                </label>
                <Textarea
                  id="external-agent-about"
                  maxLength={500}
                  onChange={(event) => setAbout(event.target.value)}
                  value={about}
                />
              </div>

              <p className="rounded-xl border border-border/70 bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
                To use this agent in another channel, add it as a channel
                member, then add that channel to the Hermes gateway’s watched
                channels (or configure Hermes to watch all joined channels).
              </p>
            </>
          ) : (
            <>
              <div className="space-y-2">
                <label
                  className="block text-sm font-medium"
                  htmlFor="external-agent-nsec"
                >
                  Private key (nsec)
                </label>
                <Input
                  autoComplete="off"
                  id="external-agent-nsec"
                  onChange={(event) => setNsec(event.target.value)}
                  placeholder="nsec1…"
                  type="password"
                  value={nsec}
                />
              </div>
              <div className="flex gap-3 rounded-xl border border-primary/20 bg-primary/5 px-3 py-3 text-sm">
                <KeyRound className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
                <p>
                  The key is verified against this agent and stored in the macOS
                  Keychain. Buzz never displays it again or changes the Hermes
                  runtime.
                </p>
              </div>
            </>
          )}
        </div>

        <div className="flex justify-end gap-2 border-t border-border/60 px-6 py-4">
          <Button
            disabled={busy}
            onClick={() => onOpenChange(false)}
            size="sm"
            type="button"
            variant="ghost"
          >
            Cancel
          </Button>
          <Button
            disabled={
              busy ||
              statusQuery.isLoading ||
              (!linked && nsec.trim().length === 0) ||
              (linked && displayName.trim().length === 0)
            }
            onClick={() => void (linked ? handleSave() : handleLink())}
            size="sm"
            type="button"
          >
            {pendingAction === "link" || pendingAction === "save" ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : null}
            {linked ? "Save profile" : "Link securely"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
