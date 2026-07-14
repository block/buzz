import * as React from "react";
import {
  AlertCircle,
  ChevronRight,
  Download,
  Link2,
  Loader2,
  X,
} from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { toast } from "sonner";

import { useEncodeAgentSnapshotForSendMutation } from "@/features/agents/hooks";
import { useOpenDmMutation } from "@/features/channels/hooks";
import { buildAgentSnapshotClipboardHtml } from "@/features/messages/lib/agentSnapshotClipboard";
import { useProfileQuery } from "@/features/profile/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { uploadMediaBytes, type BlobDescriptor } from "@/shared/api/tauri";
import { copyTextToSystemClipboard } from "@/shared/api/tauriMedia";
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

import {
  formatShareRecipientName,
  PersonaShareRecipients,
} from "./PersonaShareRecipients";
import { SnapshotOptionMenu } from "./SnapshotOptionMenu";
import { resolveSnapshotAvatarPng } from "./snapshotAvatarPng";
import { useSnapshotSendController } from "./useSnapshotSendController";

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

const RECIPIENT_ACTION_TRANSITION = {
  duration: 0.18,
  ease: [0.23, 1, 0.32, 1],
} as const;

const SHARE_WARNING_TRANSITION = {
  duration: 0.22,
  ease: [0.23, 1, 0.32, 1],
} as const;

type PendingMemoryShare = {
  action: "copy" | "send";
  memoryLevel: Exclude<SnapshotMemoryLevel, "none">;
  recipientNames?: string[];
};

function formatRecipientAudience(names: readonly string[]): string {
  if (names.length === 0) return "The people you selected";
  if (names.length === 1) return names[0] ?? "The person you selected";
  if (names.length === 2) return `${names[0]} and ${names[1]}`;
  return `${names.slice(0, -1).join(", ")}, and ${names.at(-1)}`;
}

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
  const recipientAudience = formatRecipientAudience(
    pendingShare?.recipientNames ?? [],
  );

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
              : `${recipientAudience}—and anyone with the file link—can view it.`}{" "}
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
  const snapshotSendController = useSnapshotSendController(open);
  const profileQuery = useProfileQuery(open);
  const shouldReduceMotion = useReducedMotion();
  const [selectedRecipients, setSelectedRecipients] = React.useState<
    UserSearchResult[]
  >([]);
  const [isCopying, setIsCopying] = React.useState(false);
  const [pendingMemoryShare, setPendingMemoryShare] =
    React.useState<PendingMemoryShare | null>(null);
  const [linkShareLevel, setLinkShareLevel] =
    React.useState<SnapshotMemoryLevel>("none");
  const [recipientShareLevel, setRecipientShareLevel] =
    React.useState<SnapshotMemoryLevel>("none");

  const isSending = ["preparing", "uploading", "sending"].includes(
    snapshotSendController.state.phase,
  );
  const isActionPending = isPending || isCopying || isSending;
  const hasLinkedAgent = linkedAgentPubkey !== null;
  const hasSelectedRecipients = selectedRecipients.length > 0;
  const showMemoryWarning =
    linkShareLevel !== "none" ||
    (hasSelectedRecipients && recipientShareLevel !== "none");
  const recipientActionTransition = shouldReduceMotion
    ? { duration: 0 }
    : RECIPIENT_ACTION_TRANSITION;
  const warningTransition = shouldReduceMotion
    ? { duration: 0 }
    : SHARE_WARNING_TRANSITION;
  const ownerDisplayName =
    profileQuery.data?.displayName?.trim() || "Your account";
  const excludedRecipientPubkeys = React.useMemo(
    () =>
      snapshotSendController.relaySelfPubkey
        ? [snapshotSendController.relaySelfPubkey]
        : [],
    [snapshotSendController.relaySelfPubkey],
  );

  React.useEffect(() => {
    if (open) {
      setSelectedRecipients([]);
      setIsCopying(false);
      setPendingMemoryShare(null);
      setLinkShareLevel("none");
      setRecipientShareLevel("none");
      encodeSnapshotMutation.reset();
      snapshotSendController.reset();
    }
  }, [open, encodeSnapshotMutation.reset, snapshotSendController.reset]);

  async function encodeSnapshot(memoryLevel: SnapshotMemoryLevel) {
    return encodeSnapshotMutation.mutateAsync({
      id: persona.id,
      memoryLevel: hasLinkedAgent ? memoryLevel : "none",
      format: "png",
      memorySourcePubkey: linkedAgentPubkey,
      avatarPngDataUrl: await resolveSnapshotAvatarPng(persona.avatarUrl),
    });
  }

  async function uploadSnapshot(
    memoryLevel: SnapshotMemoryLevel,
  ): Promise<BlobDescriptor> {
    const encoded = await encodeSnapshot(memoryLevel);
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
      await copyTextToSystemClipboard(
        uploaded.url,
        buildAgentSnapshotClipboardHtml({
          attachment: uploaded,
          displayName: persona.displayName,
        }),
      );
      toast.success("Link copied");
    } catch {
      toast.error("Couldn’t copy link. Try again.");
    } finally {
      setIsCopying(false);
    }
  }

  async function sendToRecipients(memoryLevel: SnapshotMemoryLevel) {
    if (isActionPending || selectedRecipients.length === 0) return;

    const sent = await snapshotSendController.beginSend(
      () => encodeSnapshot(memoryLevel),
      async () => {
        const directMessage = await openDmMutation.mutateAsync({
          pubkeys: selectedRecipients.map((recipient) => recipient.pubkey),
        });
        return directMessage.id;
      },
      persona.displayName,
    );

    if (sent) {
      toast.success(`Sent ${persona.displayName}`);
      onOpenChange(false);
    } else if (sent === false) {
      toast.error("Couldn’t send agent. Try again.");
    }
  }

  function requestMemoryShare(
    action: PendingMemoryShare["action"],
    memoryLevel: SnapshotMemoryLevel,
  ) {
    if (isActionPending) return;

    const effectiveMemoryLevel = hasLinkedAgent ? memoryLevel : "none";
    if (effectiveMemoryLevel !== "none") {
      setPendingMemoryShare({
        action,
        memoryLevel: effectiveMemoryLevel,
        recipientNames:
          action === "send"
            ? selectedRecipients.map(formatShareRecipientName)
            : undefined,
      });
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
        className="max-w-lg gap-3 bg-transparent p-0 shadow-none"
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
            className="absolute right-4 top-4 flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors duration-150 ease-out hover:bg-accent hover:text-accent-foreground focus:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
            disabled={isActionPending}
          >
            <X className="h-4 w-4" />
            <span className="sr-only">Close</span>
          </DialogClose>

          <div className="space-y-4 pt-4">
            <div className="flex items-start gap-2">
              <motion.div
                className="min-w-0 flex-1"
                layout
                transition={recipientActionTransition}
              >
                <PersonaShareRecipients
                  disabled={
                    isActionPending || !snapshotSendController.isDmSafetyReady
                  }
                  excludedPubkeys={excludedRecipientPubkeys}
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
              </motion.div>
              <AnimatePresence initial={false} mode="popLayout">
                {hasSelectedRecipients ? (
                  <motion.div
                    animate={{ opacity: 1 }}
                    className="shrink-0"
                    data-testid="persona-share-send-motion"
                    exit={{ opacity: 0 }}
                    initial={shouldReduceMotion ? false : { opacity: 0 }}
                    layout
                    transition={recipientActionTransition}
                  >
                    <Button
                      className="h-10"
                      data-testid="persona-share-send"
                      disabled={
                        isActionPending ||
                        !snapshotSendController.isDmSafetyReady
                      }
                      onClick={() =>
                        requestMemoryShare("send", recipientShareLevel)
                      }
                      type="button"
                    >
                      {isSending ? "Sending…" : "Send"}
                    </Button>
                  </motion.div>
                ) : null}
              </AnimatePresence>
            </div>

            <section
              className="space-y-3 pt-2"
              data-testid="persona-share-access"
            >
              <h3 className="text-sm font-semibold text-secondary-foreground/75">
                Who has access
              </h3>
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

            <div>
              <AnimatePresence initial={false}>
                {showMemoryWarning ? (
                  <motion.div
                    animate={{ height: "auto", opacity: 1 }}
                    className="overflow-hidden pb-4"
                    data-testid="persona-share-memory-warning-motion"
                    exit={{ height: 0, opacity: 0 }}
                    initial={{ height: 0, opacity: 0 }}
                    key="persona-share-memory-warning"
                    transition={warningTransition}
                  >
                    <div
                      className="flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-700 dark:text-amber-400"
                      data-testid="persona-share-memory-warning"
                    >
                      <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
                      <p>
                        Memory is stored as <strong>plaintext</strong> in the
                        snapshot. Only share it with people you trust.
                      </p>
                    </div>
                  </motion.div>
                ) : null}
              </AnimatePresence>

              <div
                className="flex items-center gap-4 pt-2"
                data-testid="persona-share-copy-link-footer"
              >
                <p className="min-w-0 max-w-64 flex-1 text-xs text-secondary-foreground/75">
                  Anyone with the link can duplicate and use this agent.
                </p>
                <Button
                  className="ml-auto shrink-0 shadow-none"
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
