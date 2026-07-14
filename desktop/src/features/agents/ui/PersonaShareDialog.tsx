import * as React from "react";
import { ChevronRight, Download, Link2, Loader2, X } from "lucide-react";
import { toast } from "sonner";

import { useEncodeAgentSnapshotForSendMutation } from "@/features/agents/hooks";
import { useOpenDmMutation } from "@/features/channels/hooks";
import {
  buildOutgoingMessage,
  formatImetaMediaLine,
} from "@/features/messages/lib/imetaMediaMarkdown";
import { useProfileQuery } from "@/features/profile/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import {
  sendChannelMessage,
  uploadMediaBytes,
  type BlobDescriptor,
} from "@/shared/api/tauri";
import type { SnapshotMemoryLevel } from "@/shared/api/tauriPersonas";
import type { AgentPersona, UserSearchResult } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

import { PersonaShareRecipients } from "./PersonaShareRecipients";
import { SnapshotOptionMenu } from "./SnapshotOptionMenu";
import { resolveSnapshotAvatarPng } from "./snapshotAvatarPng";

type PersonaShareDialogProps = {
  isPending: boolean;
  linkedAgentPubkey: string | null;
  onExport: () => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  persona: AgentPersona;
};

const SHARE_LEVELS: { value: SnapshotMemoryLevel; label: string }[] = [
  { value: "none", label: "Agent" },
  { value: "core", label: "Agent + core memory" },
  { value: "everything", label: "Agent + all memories" },
];

type PendingMemoryShare = {
  action: "copy" | "send";
  memoryLevel: Exclude<SnapshotMemoryLevel, "none">;
};

function MemoryShareConfirmation({
  pendingShare,
  onCancel,
  onConfirm,
}: {
  pendingShare: PendingMemoryShare | null;
  onCancel: () => void;
  onConfirm: (pendingShare: PendingMemoryShare) => void;
}) {
  const isLinkShare = pendingShare?.action === "copy";
  const memoryLabel =
    pendingShare?.memoryLevel === "core" ? "core memory" : "all memories";

  return (
    <AlertDialog
      onOpenChange={(nextOpen) => {
        if (!nextOpen) onCancel();
      }}
      open={pendingShare !== null}
    >
      <AlertDialogContent data-testid="persona-share-memory-confirmation">
        <AlertDialogHeader>
          <AlertDialogTitle>Share memories?</AlertDialogTitle>
          <AlertDialogDescription>
            This agent includes <strong>plaintext {memoryLabel}</strong>.{" "}
            {isLinkShare
              ? "Anyone with the link can view it."
              : "The people you selected—and anyone with the file link—can view it."}{" "}
            Only share with people you trust.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button type="button" variant="outline">
              Cancel
            </Button>
          </AlertDialogCancel>
          <AlertDialogAction asChild>
            <Button
              data-testid="persona-share-memory-confirm"
              onClick={() => {
                if (pendingShare) onConfirm(pendingShare);
              }}
              type="button"
            >
              {isLinkShare ? "Copy link" : "Send"}
            </Button>
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

function ShareLevelControl({
  ariaLabel,
  className,
  disabled,
  hasLinkedAgent,
  onOpenChange,
  staticClassName,
  testId,
  value,
  onChange,
}: {
  ariaLabel: string;
  className?: string;
  disabled: boolean;
  hasLinkedAgent: boolean;
  onOpenChange?: (open: boolean) => void;
  staticClassName?: string;
  testId: string;
  value: SnapshotMemoryLevel;
  onChange: (level: SnapshotMemoryLevel) => void;
}) {
  if (!hasLinkedAgent) {
    return (
      <span
        className={cn(
          "inline-flex h-8 w-auto shrink-0 items-center justify-end px-2 text-sm text-muted-foreground",
          staticClassName,
        )}
        data-testid={testId}
      >
        Agent
      </span>
    );
  }

  return (
    <SnapshotOptionMenu
      ariaLabel={ariaLabel}
      className={className}
      disabled={disabled}
      onOpenChange={onOpenChange}
      onValueChange={(nextValue) => onChange(nextValue as SnapshotMemoryLevel)}
      options={SHARE_LEVELS}
      testId={testId}
      value={value}
    />
  );
}

export function PersonaShareDialog({
  isPending,
  linkedAgentPubkey,
  onExport,
  onOpenChange,
  open,
  persona,
}: PersonaShareDialogProps) {
  const encodeSnapshotMutation = useEncodeAgentSnapshotForSendMutation();
  const openDmMutation = useOpenDmMutation();
  const profileQuery = useProfileQuery(open);
  const [selectedRecipients, setSelectedRecipients] = React.useState<
    UserSearchResult[]
  >([]);
  const [isCopying, setIsCopying] = React.useState(false);
  const [isSending, setIsSending] = React.useState(false);
  const [pendingMemoryShare, setPendingMemoryShare] =
    React.useState<PendingMemoryShare | null>(null);
  const [linkShareLevel, setLinkShareLevel] =
    React.useState<SnapshotMemoryLevel>("none");
  const [recipientShareLevel, setRecipientShareLevel] =
    React.useState<SnapshotMemoryLevel>("none");

  const isActionPending = isPending || isCopying || isSending;
  const hasLinkedAgent = linkedAgentPubkey !== null;
  const ownerDisplayName =
    profileQuery.data?.displayName?.trim() || "Your account";

  React.useEffect(() => {
    if (open) {
      setSelectedRecipients([]);
      setIsCopying(false);
      setIsSending(false);
      setPendingMemoryShare(null);
      setLinkShareLevel("none");
      setRecipientShareLevel("none");
      encodeSnapshotMutation.reset();
    }
  }, [open, encodeSnapshotMutation.reset]);

  async function uploadSnapshot(
    memoryLevel: SnapshotMemoryLevel,
  ): Promise<BlobDescriptor> {
    const encoded = await encodeSnapshotMutation.mutateAsync({
      id: persona.id,
      memoryLevel: hasLinkedAgent ? memoryLevel : "none",
      format: "png",
      memorySourcePubkey: linkedAgentPubkey,
      avatarPngDataUrl: await resolveSnapshotAvatarPng(persona.avatarUrl),
    });
    const uploaded = await uploadMediaBytes(
      encoded.fileBytes,
      encoded.fileName,
    );
    const { thumb: _thumb, ...uploadedWithoutThumb } = uploaded;

    return {
      ...uploadedWithoutThumb,
      filename: encoded.fileName,
    };
  }

  async function copyLink(memoryLevel: SnapshotMemoryLevel) {
    if (isActionPending) return;

    setIsCopying(true);
    try {
      const uploaded = await uploadSnapshot(memoryLevel);
      await navigator.clipboard.writeText(uploaded.url);
      toast.success("Link copied");
    } catch {
      toast.error("Couldn’t copy link");
    } finally {
      setIsCopying(false);
    }
  }

  async function sendToRecipients(memoryLevel: SnapshotMemoryLevel) {
    if (isActionPending || selectedRecipients.length === 0) return;

    setIsSending(true);
    try {
      const directMessage = await openDmMutation.mutateAsync({
        pubkeys: selectedRecipients.map((recipient) => recipient.pubkey),
      });
      const uploaded = await uploadSnapshot(memoryLevel);
      const outgoingMessage = buildOutgoingMessage("", [uploaded]);
      await sendChannelMessage(
        directMessage.id,
        formatImetaMediaLine(uploaded, { label: persona.displayName }),
        null,
        outgoingMessage.mediaTags ?? [],
      );
      toast.success(`Sent ${persona.displayName}`);
      onOpenChange(false);
    } catch {
      toast.error("Couldn’t send agent. Try again.");
    } finally {
      setIsSending(false);
    }
  }

  function requestMemoryShare(
    action: PendingMemoryShare["action"],
    memoryLevel: SnapshotMemoryLevel,
  ) {
    if (isActionPending) return;

    const effectiveMemoryLevel = hasLinkedAgent ? memoryLevel : "none";
    if (effectiveMemoryLevel !== "none") {
      setPendingMemoryShare({ action, memoryLevel: effectiveMemoryLevel });
      return;
    }

    if (action === "copy") {
      void copyLink("none");
    } else {
      void sendToRecipients("none");
    }
  }

  function confirmMemoryShare(pendingShare: PendingMemoryShare) {
    setPendingMemoryShare(null);
    if (pendingShare.action === "copy") {
      void copyLink(pendingShare.memoryLevel);
    } else {
      void sendToRecipients(pendingShare.memoryLevel);
    }
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        aria-describedby={undefined}
        className="max-w-lg gap-4 bg-transparent p-0 shadow-none"
        data-testid="persona-share-dialog"
        showCloseButton={false}
      >
        <div
          className="relative rounded-2xl bg-background p-6 shadow-2xl"
          data-testid="persona-share-main-card"
        >
          <DialogHeader className="space-y-0">
            <DialogTitle className="min-w-0 truncate pr-10">
              Share {persona.displayName}
            </DialogTitle>
          </DialogHeader>
          <DialogClose
            className="absolute right-4 top-4 flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors duration-150 ease-out hover:bg-accent hover:text-accent-foreground focus:outline-hidden focus:ring-1 focus:ring-ring disabled:pointer-events-none disabled:opacity-50"
            disabled={isActionPending}
          >
            <X className="h-4 w-4" />
            <span className="sr-only">Close</span>
          </DialogClose>

          <div className="space-y-4 pt-4">
            <div className="flex items-start gap-2">
              <PersonaShareRecipients
                disabled={isActionPending}
                onSelectionChange={setSelectedRecipients}
                open={open}
                renderEndControl={(handleAccessOpenChange) => (
                  <ShareLevelControl
                    ariaLabel="Recipient access"
                    className="h-7"
                    disabled={isActionPending}
                    hasLinkedAgent={hasLinkedAgent}
                    onChange={setRecipientShareLevel}
                    onOpenChange={handleAccessOpenChange}
                    staticClassName="h-7 w-auto"
                    testId="persona-share-recipient-access"
                    value={recipientShareLevel}
                  />
                )}
                selectedUsers={selectedRecipients}
              />
              <Button
                className="h-10 shrink-0"
                data-testid="persona-share-send"
                disabled={isActionPending || selectedRecipients.length === 0}
                onClick={() => requestMemoryShare("send", recipientShareLevel)}
                type="button"
              >
                {isSending ? "Sending…" : "Send"}
              </Button>
            </div>

            <section className="space-y-3" data-testid="persona-share-access">
              <h3 className="text-sm font-semibold">Who has access</h3>
              <div
                className="flex min-h-9 items-center gap-3"
                data-testid="persona-share-access-link"
              >
                <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
                  <Link2 className="h-4 w-4" />
                </span>
                <span className="min-w-0 flex-1 truncate text-sm font-medium">
                  Anyone with a link
                </span>
                <ShareLevelControl
                  ariaLabel="Link access"
                  className="-mr-2"
                  disabled={isActionPending}
                  hasLinkedAgent={hasLinkedAgent}
                  onChange={setLinkShareLevel}
                  testId="persona-share-link-access"
                  value={linkShareLevel}
                />
              </div>

              <div
                className="flex min-h-9 items-center gap-3"
                data-testid="persona-share-access-owner"
              >
                <ProfileAvatar
                  avatarUrl={profileQuery.data?.avatarUrl ?? null}
                  className="h-8 w-8 text-xs shadow-none"
                  iconClassName="h-4 w-4"
                  label={ownerDisplayName}
                />
                <div className="flex min-w-0 flex-1 items-baseline gap-1.5">
                  <span className="truncate text-sm font-medium">
                    {ownerDisplayName}
                  </span>
                  <span className="shrink-0 text-xs text-muted-foreground">
                    (You)
                  </span>
                </div>
                <span className="shrink-0 text-sm text-muted-foreground">
                  Owner
                </span>
              </div>
            </section>

            <div
              className="flex items-center gap-4 pt-2"
              data-testid="persona-share-copy-link-footer"
            >
              <p className="min-w-0 max-w-64 flex-1 text-xs text-secondary-foreground/75">
                Anyone in this workspace with the link can duplicate and use
                this agent.
              </p>
              <Button
                className="ml-auto shrink-0"
                data-testid="persona-share-copy-link"
                disabled={isActionPending}
                onClick={() => requestMemoryShare("copy", linkShareLevel)}
                size="sm"
                type="button"
              >
                {isCopying ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Link2 className="h-4 w-4" />
                )}
                {isCopying ? "Copying…" : "Copy link"}
              </Button>
            </div>
          </div>
        </div>
        <button
          className="relative flex min-h-14 w-full items-center gap-3 rounded-2xl bg-background px-5 py-4 text-left text-sm font-medium shadow-2xl outline-hidden transition-colors hover:bg-muted focus-visible:bg-muted disabled:cursor-not-allowed disabled:opacity-60"
          data-testid="persona-share-export"
          disabled={isActionPending}
          onClick={onExport}
          type="button"
        >
          <Download className="h-4 w-4 shrink-0 text-muted-foreground" />
          <span className="min-w-0 flex-1">Export agent</span>
          <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
        </button>
      </DialogContent>
      <MemoryShareConfirmation
        onCancel={() => setPendingMemoryShare(null)}
        onConfirm={confirmMemoryShare}
        pendingShare={pendingMemoryShare}
      />
    </Dialog>
  );
}
