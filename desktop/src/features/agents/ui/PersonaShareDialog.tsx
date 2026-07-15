import * as React from "react";
import {
  AlertCircle,
  Check,
  ChevronRight,
  Download,
  Link2,
  X,
} from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { toast } from "sonner";

import { useEncodeAgentSnapshotForSendMutation } from "@/features/agents/hooks";
import {
  useOpenDmMutation,
  useUpsertCachedChannel,
} from "@/features/channels/hooks";
import { buildSnapshotClipboardHtml } from "@/features/messages/lib/agentSnapshotClipboard";
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
import { Separator } from "@/shared/ui/separator";
import { Spinner } from "@/shared/ui/spinner";

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

type SnapshotShareDialogProps = {
  displayName: string;
  encodeSnapshot: (
    memoryLevel: SnapshotMemoryLevel,
  ) => Promise<{ fileBytes: number[]; fileName: string }>;
  hasMemoryOptions: boolean;
  isPending: boolean;
  onExport: () => void;
  onOpenChange: (open: boolean) => void;
  onReset?: () => void;
  open: boolean;
  snapshotKind: "agent" | "team";
  testIdPrefix: string;
};

type EncodedSnapshot = {
  fileBytes: number[];
  fileName: string;
};

const RECIPIENT_ACTION_TRANSITION = {
  duration: 0.18,
  ease: [0.23, 1, 0.32, 1],
} as const;

const SHARE_WARNING_TRANSITION = {
  duration: 0.22,
  ease: [0.23, 1, 0.32, 1],
} as const;

const COPY_FEEDBACK_TRANSITION = {
  duration: 0.14,
  ease: [0.23, 1, 0.32, 1],
} as const;

const COPY_BUTTON_LAYOUT_TRANSITION = {
  duration: 0.2,
  ease: [0.23, 1, 0.32, 1],
} as const;

const COPY_FEEDBACK_RESET_MS = 1500;

type CopyStatus = "idle" | "copying" | "copied";

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
  itemLabel,
  pendingShare,
  onCancel,
  onConfirm,
  testIdPrefix,
}: {
  itemLabel: string;
  pendingShare: PendingMemoryShare | null;
  onCancel: () => void;
  onConfirm: (pendingShare: PendingMemoryShare) => void;
  testIdPrefix: string;
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
      <AlertDialogContent data-testid={`${testIdPrefix}-memory-confirmation`}>
        <AlertDialogHeader>
          <AlertDialogTitle>Share memories?</AlertDialogTitle>
          <AlertDialogDescription>
            This {itemLabel} includes <strong>plaintext {memoryLabel}</strong>.{" "}
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
              data-testid={`${testIdPrefix}-memory-confirm`}
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
  hasMemoryOptions,
  onOpenChange,
  staticClassName,
  staticLabel,
  testId,
  value,
  options,
  onChange,
}: {
  ariaLabel: string;
  className?: string;
  disabled: boolean;
  hasMemoryOptions: boolean;
  onOpenChange?: (open: boolean) => void;
  staticClassName?: string;
  staticLabel: string;
  testId: string;
  value: SnapshotMemoryLevel;
  options: { value: SnapshotMemoryLevel; label: string }[];
  onChange: (level: SnapshotMemoryLevel) => void;
}) {
  if (!hasMemoryOptions) {
    return (
      <span
        className={cn(
          "inline-flex h-8 w-auto shrink-0 items-center justify-end px-2 text-sm text-muted-foreground",
          staticClassName,
        )}
        data-testid={testId}
      >
        {staticLabel}
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
      options={options}
      testId={testId}
      value={value}
    />
  );
}

export function SnapshotShareDialog({
  displayName,
  encodeSnapshot,
  hasMemoryOptions,
  isPending,
  onExport,
  onOpenChange,
  onReset,
  open,
  snapshotKind,
  testIdPrefix,
}: SnapshotShareDialogProps) {
  const openDmMutation = useOpenDmMutation();
  const upsertCachedChannel = useUpsertCachedChannel();
  const snapshotSendController = useSnapshotSendController(open);
  const shouldReduceMotion = useReducedMotion();
  const [selectedRecipients, setSelectedRecipients] = React.useState<
    UserSearchResult[]
  >([]);
  const [copyStatus, setCopyStatus] = React.useState<CopyStatus>("idle");
  const [pendingMemoryShare, setPendingMemoryShare] =
    React.useState<PendingMemoryShare | null>(null);
  const [linkShareLevel, setLinkShareLevel] =
    React.useState<SnapshotMemoryLevel>("none");
  const [recipientShareLevel, setRecipientShareLevel] =
    React.useState<SnapshotMemoryLevel>("none");
  const encodedSnapshotCacheRef = React.useRef(
    new Map<SnapshotMemoryLevel, Promise<EncodedSnapshot>>(),
  );

  const isSending = ["preparing", "uploading", "sending"].includes(
    snapshotSendController.state.phase,
  );
  const isCopying = copyStatus === "copying";
  const copyStatusLabel =
    copyStatus === "copying"
      ? "Copying…"
      : copyStatus === "copied"
        ? "Copied"
        : "Copy link";
  const isActionPending = isPending || isCopying || isSending;
  const isInterfacePending = isPending || isSending;
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
  const copyFeedbackTransition = shouldReduceMotion
    ? { duration: 0 }
    : COPY_FEEDBACK_TRANSITION;
  const copyButtonLayoutTransition = shouldReduceMotion
    ? { duration: 0 }
    : COPY_BUTTON_LAYOUT_TRANSITION;
  const excludedRecipientPubkeys = React.useMemo(
    () =>
      snapshotSendController.relaySelfPubkey
        ? [snapshotSendController.relaySelfPubkey]
        : [],
    [snapshotSendController.relaySelfPubkey],
  );
  const itemLabel = snapshotKind === "team" ? "team" : "agent";
  const itemLabelTitle = snapshotKind === "team" ? "Team" : "Agent";
  const shareLevels = React.useMemo(
    () => [
      { value: "none" as const, label: `${itemLabelTitle} only` },
      {
        value: "core" as const,
        label: `${itemLabelTitle} + core memory`,
      },
      {
        value: "everything" as const,
        label: `${itemLabelTitle} + all memories`,
      },
    ],
    [itemLabelTitle],
  );
  const getEncodedSnapshot = React.useCallback(
    (memoryLevel: SnapshotMemoryLevel) => {
      const effectiveMemoryLevel = hasMemoryOptions ? memoryLevel : "none";
      const cached = encodedSnapshotCacheRef.current.get(effectiveMemoryLevel);
      if (cached) return cached;

      const pending = encodeSnapshot(effectiveMemoryLevel).catch((error) => {
        if (
          encodedSnapshotCacheRef.current.get(effectiveMemoryLevel) === pending
        ) {
          encodedSnapshotCacheRef.current.delete(effectiveMemoryLevel);
        }
        throw error;
      });
      encodedSnapshotCacheRef.current.set(effectiveMemoryLevel, pending);
      return pending;
    },
    [encodeSnapshot, hasMemoryOptions],
  );

  React.useEffect(() => {
    if (open) {
      encodedSnapshotCacheRef.current.clear();
      setSelectedRecipients([]);
      setCopyStatus("idle");
      setPendingMemoryShare(null);
      setLinkShareLevel("none");
      setRecipientShareLevel("none");
      onReset?.();
      snapshotSendController.reset();
      // Encoding is local and carries no memory at this level. Starting it in
      // the background removes most of the preparation delay from the common
      // Copy link and Send paths without uploading anything before intent.
      void getEncodedSnapshot("none").catch(() => undefined);
    }
  }, [getEncodedSnapshot, open, onReset, snapshotSendController.reset]);

  React.useEffect(() => {
    if (copyStatus !== "copied") return;

    const resetTimer = window.setTimeout(
      () => setCopyStatus("idle"),
      COPY_FEEDBACK_RESET_MS,
    );
    return () => window.clearTimeout(resetTimer);
  }, [copyStatus]);

  async function uploadSnapshot(
    memoryLevel: SnapshotMemoryLevel,
  ): Promise<BlobDescriptor> {
    const encoded = await getEncodedSnapshot(memoryLevel);
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

    setCopyStatus("copying");
    try {
      const uploaded = await uploadSnapshot(memoryLevel);
      await copyTextToSystemClipboard(
        uploaded.url,
        buildSnapshotClipboardHtml({
          attachment: uploaded,
          displayName,
          snapshotKind,
        }),
      );
      setCopyStatus("copied");
    } catch {
      setCopyStatus("idle");
      toast.error("Couldn’t copy link. Try again.");
    }
  }

  async function sendToRecipients(memoryLevel: SnapshotMemoryLevel) {
    if (isActionPending || selectedRecipients.length === 0) return;

    const sent = await snapshotSendController.beginSend(
      () => getEncodedSnapshot(memoryLevel),
      async () => {
        const directMessage = await openDmMutation.mutateAsync({
          pubkeys: selectedRecipients.map((recipient) => recipient.pubkey),
        });
        await upsertCachedChannel(directMessage);
        return directMessage.id;
      },
      displayName,
    );

    if (sent) {
      toast.success(`Sent a copy of ${displayName}`);
      onOpenChange(false);
    } else if (sent === false) {
      toast.error(`Couldn’t send ${itemLabel}. Try again.`);
    }
  }

  function requestMemoryShare(
    action: PendingMemoryShare["action"],
    memoryLevel: SnapshotMemoryLevel,
  ) {
    if (isActionPending) return;

    const effectiveMemoryLevel = hasMemoryOptions ? memoryLevel : "none";
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

  function handleDialogOpenChange(nextOpen: boolean) {
    if (!nextOpen && isInterfacePending) return;
    onOpenChange(nextOpen);
  }

  return (
    <Dialog onOpenChange={handleDialogOpenChange} open={open}>
      <DialogContent
        aria-describedby={undefined}
        className="max-w-xl gap-3 bg-transparent p-0 shadow-none"
        data-testid={`${testIdPrefix}-dialog`}
        showCloseButton={false}
      >
        <div
          className="relative rounded-2xl bg-background p-6 pb-4 shadow-2xl"
          data-testid={`${testIdPrefix}-main-card`}
        >
          <DialogHeader className="space-y-0">
            <DialogTitle className="min-w-0 truncate pr-10">
              Share {displayName}
            </DialogTitle>
          </DialogHeader>
          <DialogClose
            className="absolute right-4 top-4 flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors duration-150 ease-out hover:bg-accent hover:text-accent-foreground focus:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
            disabled={isInterfacePending}
          >
            <X className="h-4 w-4" />
            <span className="sr-only">Close</span>
          </DialogClose>

          <div className="space-y-4 pt-4">
            <div className="space-y-2">
              <div className="flex items-start gap-2">
                <motion.div
                  className="min-w-0 flex-1"
                  layout
                  transition={recipientActionTransition}
                >
                  <PersonaShareRecipients
                    disabled={
                      isInterfacePending ||
                      !snapshotSendController.isDmSafetyReady
                    }
                    excludedPubkeys={excludedRecipientPubkeys}
                    onSelectionChange={setSelectedRecipients}
                    open={open}
                    renderEndControl={(handleAccessOpenChange) => (
                      <ShareLevelControl
                        ariaLabel="What to include"
                        className="-mr-2 h-7"
                        disabled={isInterfacePending}
                        hasMemoryOptions={hasMemoryOptions}
                        onChange={setRecipientShareLevel}
                        onOpenChange={handleAccessOpenChange}
                        options={shareLevels}
                        staticLabel={`${itemLabelTitle} only`}
                        staticClassName="-mr-2 h-7 w-auto"
                        testId={`${testIdPrefix}-recipient-access`}
                        value={recipientShareLevel}
                      />
                    )}
                    selectedUsers={selectedRecipients}
                    testIdPrefix={testIdPrefix}
                  />
                </motion.div>
                <AnimatePresence initial={false} mode="popLayout">
                  {hasSelectedRecipients ? (
                    <motion.div
                      animate={{ opacity: 1 }}
                      className="shrink-0"
                      data-testid={`${testIdPrefix}-send-motion`}
                      exit={{ opacity: 0 }}
                      initial={shouldReduceMotion ? false : { opacity: 0 }}
                      layout
                      transition={recipientActionTransition}
                    >
                      <Button
                        className="h-10"
                        data-testid={`${testIdPrefix}-send`}
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
              <p
                className="text-xs text-secondary-foreground/75"
                data-testid={`${testIdPrefix}-send-description`}
              >
                They’ll receive a copy they can add and use. Changes you make
                later won’t sync.
              </p>
            </div>

            <AnimatePresence initial={false}>
              {showMemoryWarning ? (
                <motion.div
                  animate={{ height: "auto", opacity: 1 }}
                  className="overflow-hidden"
                  data-testid={`${testIdPrefix}-memory-warning-motion`}
                  exit={{ height: 0, opacity: 0 }}
                  initial={{ height: 0, opacity: 0 }}
                  key={`${testIdPrefix}-memory-warning`}
                  transition={warningTransition}
                >
                  <div
                    className="flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-700 dark:text-amber-400"
                    data-testid={`${testIdPrefix}-memory-warning`}
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

            <section
              className="pt-6"
              data-testid={`${testIdPrefix}-copy-link-footer`}
            >
              <div
                className="flex items-center gap-3"
                data-testid={`${testIdPrefix}-link-row`}
              >
                <span
                  className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground"
                  data-testid={`${testIdPrefix}-link-icon`}
                >
                  <Link2 className="h-4 w-4" />
                </span>
                <div
                  className="min-w-0 flex-1"
                  data-testid={`${testIdPrefix}-link-copy`}
                >
                  <h3 className="text-sm font-medium">Share with a link</h3>
                  <p className="text-xs text-secondary-foreground/75">
                    Anyone with the link can add and use a copy.
                  </p>
                </div>
                <ShareLevelControl
                  ariaLabel="What to include in the link"
                  disabled={isInterfacePending}
                  hasMemoryOptions={hasMemoryOptions}
                  onChange={setLinkShareLevel}
                  options={shareLevels}
                  staticLabel={`${itemLabelTitle} only`}
                  testId={`${testIdPrefix}-link-access`}
                  value={linkShareLevel}
                />
              </div>
              <Separator
                className="my-4 bg-input/40"
                data-testid={`${testIdPrefix}-link-divider`}
              />
              <div className="flex justify-end">
                <Button
                  asChild
                  className="shrink-0 border-border shadow-none disabled:opacity-100"
                  data-copy-status={copyStatus}
                  data-testid={`${testIdPrefix}-copy-link`}
                  disabled={isActionPending}
                  onClick={() => requestMemoryShare("copy", linkShareLevel)}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  <motion.button
                    layout={!shouldReduceMotion}
                    layoutDependency={copyStatus}
                    style={{ transformOrigin: "100% 50%" }}
                    transition={copyButtonLayoutTransition}
                  >
                    <span aria-live="polite" className="sr-only">
                      {copyStatusLabel}
                    </span>
                    <span
                      aria-hidden="true"
                      className="relative grid place-items-center"
                      data-testid={`${testIdPrefix}-copy-link-stage`}
                    >
                      <AnimatePresence initial={false} mode="popLayout">
                        <motion.span
                          animate={{
                            filter: "blur(0px)",
                            opacity: 1,
                            transform: "scale(1)",
                          }}
                          className="col-start-1 row-start-1 flex items-center justify-center gap-1.5 [transform-origin:50%_50%] will-change-[transform,opacity,filter]"
                          data-testid={`${testIdPrefix}-copy-link-state`}
                          exit={
                            shouldReduceMotion
                              ? { opacity: 0 }
                              : {
                                  filter: "blur(2px)",
                                  opacity: 0,
                                  transform: "scale(0.97)",
                                }
                          }
                          initial={
                            shouldReduceMotion
                              ? false
                              : {
                                  filter: "blur(2px)",
                                  opacity: 0,
                                  transform: "scale(0.97)",
                                }
                          }
                          key={copyStatus}
                          transition={copyFeedbackTransition}
                        >
                          {copyStatus === "copying" ? (
                            <Spinner
                              aria-hidden="true"
                              className="h-4 w-4 border-2"
                            />
                          ) : copyStatus === "copied" ? (
                            <Check aria-hidden="true" className="h-4 w-4" />
                          ) : (
                            <Link2 aria-hidden="true" className="h-4 w-4" />
                          )}
                          <span>{copyStatusLabel}</span>
                        </motion.span>
                      </AnimatePresence>
                    </span>
                  </motion.button>
                </Button>
              </div>
            </section>
          </div>
        </div>
        <button
          className="relative flex min-h-14 w-full items-center gap-3 rounded-2xl bg-background px-5 py-4 text-left text-sm font-medium shadow-2xl outline-hidden transition-colors hover:bg-muted focus-visible:bg-muted disabled:cursor-not-allowed disabled:opacity-60"
          data-testid={`${testIdPrefix}-export`}
          disabled={isPending || isSending}
          onClick={onExport}
          type="button"
        >
          <Download className="h-4 w-4 shrink-0 text-muted-foreground" />
          <span className="min-w-0 flex-1">Export {itemLabel}</span>
          <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
        </button>
      </DialogContent>
      <MemoryShareConfirmation
        itemLabel={itemLabel}
        onCancel={() => setPendingMemoryShare(null)}
        onConfirm={confirmMemoryShare}
        pendingShare={pendingMemoryShare}
        testIdPrefix={testIdPrefix}
      />
    </Dialog>
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
  const encodeSnapshot = React.useCallback(
    async (memoryLevel: SnapshotMemoryLevel) =>
      encodeSnapshotMutation.mutateAsync({
        id: persona.id,
        memoryLevel: linkedAgentPubkey ? memoryLevel : "none",
        format: "png",
        memorySourcePubkey: linkedAgentPubkey,
        avatarPngDataUrl: await resolveSnapshotAvatarPng(persona.avatarUrl),
      }),
    [
      encodeSnapshotMutation.mutateAsync,
      linkedAgentPubkey,
      persona.avatarUrl,
      persona.id,
    ],
  );

  return (
    <SnapshotShareDialog
      displayName={persona.displayName}
      encodeSnapshot={encodeSnapshot}
      hasMemoryOptions={linkedAgentPubkey !== null}
      isPending={isPending}
      onExport={onExport}
      onOpenChange={onOpenChange}
      onReset={encodeSnapshotMutation.reset}
      open={open}
      snapshotKind="agent"
      testIdPrefix="persona-share"
    />
  );
}
