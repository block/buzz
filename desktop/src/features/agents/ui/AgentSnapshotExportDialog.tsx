import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  AlertCircle,
  Brain,
  Download,
  ExternalLink,
  FileType2,
  KeyRound,
  Loader2,
  Sparkles,
  X,
} from "lucide-react";
import {
  AnimatePresence,
  motion,
  useAnimationControls,
  useReducedMotion,
} from "motion/react";
import { flushSync } from "react-dom";
import { toast } from "sonner";

import tradingCardTemplateUrl from "@/features/agents/assets/buzz-trading-card-template.svg";
import {
  dismissCardMintJob,
  INVALID_OPENAI_KEY_MESSAGE,
  startCardMint,
  useCardMintJobs,
} from "@/features/agents/cardMintStore";
import type {
  CardMintKeyLayer,
  MintedAgentCard,
  SnapshotFormat,
  SnapshotMemoryLevel,
} from "@/shared/api/tauriPersonas";
import {
  cardMintKeyStatus,
  cardMintSaveOpenaiKey,
  saveAgentCard,
} from "@/shared/api/tauriPersonas";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import { AgentCustomCardDraftPreview } from "./AgentCustomCardDraftPreview";
import {
  AgentTradingCardPreview,
  type AgentTradingCardAppearance,
} from "./AgentTradingCardPreview";
import { SnapshotOptionMenu } from "./SnapshotOptionMenu";
import { rasterizeInlineSvgAvatarForCard } from "./agentCardAvatar";
import { renderAgentTradingCardPng } from "./agentTradingCardExport";

type AgentSnapshotExportDialogProps = {
  agentId: string;
  agentAvatarUrl: string | null;
  agentName: string;
  isSavePending: boolean;
  open: boolean;
  /** Pubkey of the linked agent instance to use as the memory source.
   *  When null, memory levels are disabled — the definition has no agent
   *  instance with a keypair to read memory from. */
  linkedAgentPubkey: string | null;
  onSaveFile: (
    memoryLevel: SnapshotMemoryLevel,
    format: SnapshotFormat,
    cardPngDataUrl?: string,
  ) => void;
  onOpenChange: (open: boolean) => void;
};

const MEMORY_LEVELS: {
  value: SnapshotMemoryLevel;
  label: string;
}[] = [
  {
    value: "none",
    label: "Agent only",
  },
  {
    value: "core",
    label: "Agent + core memory",
  },
  {
    value: "everything",
    label: "Agent + all memories",
  },
];

const FORMAT_OPTIONS: { value: SnapshotFormat; label: string }[] = [
  { value: "json", label: "JSON" },
  { value: "png", label: "PNG" },
];

const OPENAI_KEYS_URL = "https://platform.openai.com/api-keys";

const CARD_MODE_TRANSITION = {
  duration: 1,
  ease: [0.23, 1, 0.32, 1],
} as const;

const CARD_CONTENT_TRANSITION = {
  duration: 0.22,
  ease: [0.23, 1, 0.32, 1],
} as const;

const GENERATED_CARD_REVEAL_TRANSITION = {
  ...CARD_MODE_TRANSITION,
  delay: 0.24,
} as const;

export function AgentSnapshotExportDialog({
  agentId,
  agentAvatarUrl,
  agentName,
  isSavePending,
  open,
  linkedAgentPubkey,
  onSaveFile,
  onOpenChange,
}: AgentSnapshotExportDialogProps) {
  const [memoryLevel, setMemoryLevel] =
    React.useState<SnapshotMemoryLevel>("none");
  const [format, setFormat] = React.useState<SnapshotFormat>("png");
  const [mode, setMode] = React.useState<"default" | "custom">("default");
  const [visibleCardMode, setVisibleCardMode] = React.useState<
    "default" | "custom"
  >("default");
  const [customDescription, setCustomDescription] = React.useState("");
  const [customJobId, setCustomJobId] = React.useState<string | null>(null);
  const [customCardRevealKey, setCustomCardRevealKey] = React.useState<
    string | null
  >(null);
  const [activeCustomCard, setActiveCustomCard] =
    React.useState<MintedAgentCard | null>(null);
  const [keyDraft, setKeyDraft] = React.useState("");
  const [editingKey, setEditingKey] = React.useState(false);
  const [isPreparingAvatar, setIsPreparingAvatar] = React.useState(false);
  const [isPreparingExport, setIsPreparingExport] = React.useState(false);
  const [defaultCardAppearance, setDefaultCardAppearance] =
    React.useState<AgentTradingCardAppearance | null>(null);
  const [avatarPreparationError, setAvatarPreparationError] = React.useState<
    string | null
  >(null);
  const shouldReduceMotion = useReducedMotion();
  const cardModeControls = useAnimationControls();
  const visibleCardModeRef = React.useRef<"default" | "custom">("default");
  const [modalSurfaceElement, setModalSurfaceElement] =
    React.useState<HTMLDivElement | null>(null);
  const [modalContentsElement, setModalContentsElement] =
    React.useState<HTMLDivElement | null>(null);
  const fixedModalHeightRef = React.useRef<number | null>(null);
  const queryClient = useQueryClient();
  const cardMintJobs = useCardMintJobs();
  const customJob = customJobId
    ? cardMintJobs.find((job) => job.jobId === customJobId)
    : undefined;
  const completedCustomCard =
    customJob?.phase === "done" ? customJob.card : null;
  const displayedCustomCard =
    completedCustomCard ?? (mode === "default" ? activeCustomCard : null);
  const customCardImageUrl = displayedCustomCard
    ? `data:image/png;base64,${displayedCustomCard.cardPngBase64}`
    : undefined;
  const needsOpenAIKeyReplacement =
    customJob?.phase === "error" &&
    customJob.error === INVALID_OPENAI_KEY_MESSAGE;

  const hasLinkedAgent = linkedAgentPubkey !== null;
  const showMemoryWarning = memoryLevel !== "none";
  const compactForInlineDetails =
    (mode === "default" && showMemoryWarning) ||
    (mode === "custom" && editingKey);
  const isCreatingCustomCard =
    isPreparingAvatar || customJob?.phase === "minting";
  const keyStatusQuery = useQuery({
    enabled: open && mode === "custom",
    queryKey: ["cardMintKeyStatus", agentId],
    queryFn: () => cardMintKeyStatus(agentId),
  });
  const keyLayer: CardMintKeyLayer | undefined = keyStatusQuery.data;
  const isCheckingKey = keyStatusQuery.isPending;
  const cardContentTransition = shouldReduceMotion
    ? { duration: 0 }
    : CARD_CONTENT_TRANSITION;

  const saveCustomCardMutation = useMutation({
    mutationFn: async () => {
      if (!activeCustomCard) return false;
      return saveAgentCard(
        activeCustomCard.cardPngBase64,
        activeCustomCard.fileName,
      );
    },
    onSuccess: (saved) => {
      if (!saved) return;
      toast.success(`Exported ${agentName}'s custom card.`);
      onOpenChange(false);
    },
    onError: (error: Error) => toast.error(error.message),
  });

  const saveKeyMutation = useMutation({
    mutationFn: (key: string) => cardMintSaveOpenaiKey(key),
    onSuccess: () => {
      queryClient.setQueryData<CardMintKeyLayer>(
        ["cardMintKeyStatus", agentId],
        "card",
      );
      void queryClient.invalidateQueries({
        queryKey: ["cardMintDedicatedKeyStatus"],
      });
      if (customJobId) dismissCardMintJob(customJobId);
      setCustomJobId(null);
      setKeyDraft("");
      setEditingKey(false);
      toast.success("OpenAI key saved. You can create your card now.");
    },
    onError: (error) =>
      toast.error(
        typeof error === "string" ? error : "Couldn't save the OpenAI key.",
      ),
  });

  React.useEffect(() => {
    if (!needsOpenAIKeyReplacement) return;
    setEditingKey(true);
  }, [needsOpenAIKeyReplacement]);

  React.useLayoutEffect(() => {
    if (!completedCustomCard || !customJobId) return;

    setActiveCustomCard(completedCustomCard);
    setMemoryLevel(completedCustomCard.memoryLevel);
    setFormat("png");
    setCustomDescription("");
    setMode("default");
    setCustomCardRevealKey(customJobId);
    dismissCardMintJob(customJobId);
    setCustomJobId(null);

    visibleCardModeRef.current = "custom";
    setVisibleCardMode("custom");
    if (shouldReduceMotion) {
      void cardModeControls.set({ rotateY: 0, scale: 1 });
      return;
    }
    void cardModeControls.start(
      { rotateY: 180, scale: 1 },
      GENERATED_CARD_REVEAL_TRANSITION,
    );
  }, [cardModeControls, completedCustomCard, customJobId, shouldReduceMotion]);

  // Reset before paint as the dialog closes so a previously visible custom
  // face can never become the dialog's first frame on the next export.
  React.useLayoutEffect(() => {
    if (open) return;
    setMemoryLevel("none");
    setFormat("png");
    setMode("default");
    visibleCardModeRef.current = "default";
    setVisibleCardMode("default");
    setCustomDescription("");
    setCustomJobId(null);
    setCustomCardRevealKey(null);
    setActiveCustomCard(null);
    setKeyDraft("");
    setEditingKey(false);
    setIsPreparingAvatar(false);
    setIsPreparingExport(false);
    setDefaultCardAppearance(null);
    setAvatarPreparationError(null);
    void cardModeControls.set({ rotateY: 0, scale: 1 });
  }, [cardModeControls, open]);

  React.useLayoutEffect(() => {
    if (!open) {
      fixedModalHeightRef.current = null;
      modalSurfaceElement?.style.removeProperty("height");
      return;
    }

    if (!modalContentsElement || !modalSurfaceElement) return;
    if (fixedModalHeightRef.current === null) {
      fixedModalHeightRef.current = modalContentsElement.scrollHeight;
    }
    modalSurfaceElement.style.height = `${fixedModalHeightRef.current}px`;
  }, [modalContentsElement, modalSurfaceElement, open]);

  React.useEffect(() => {
    if (!shouldReduceMotion) return;
    const nextVisibleMode =
      mode === "custom" || activeCustomCard ? "custom" : "default";
    visibleCardModeRef.current = nextVisibleMode;
    setVisibleCardMode(nextVisibleMode);
    void cardModeControls.set({
      rotateY: 0,
      scale: mode === "custom" ? 0.8 : 1,
    });
  }, [activeCustomCard, cardModeControls, mode, shouldReduceMotion]);

  const isPending =
    isSavePending ||
    saveCustomCardMutation.isPending ||
    saveKeyMutation.isPending ||
    isPreparingAvatar ||
    isPreparingExport ||
    (mode === "default" &&
      !activeCustomCard &&
      format === "png" &&
      defaultCardAppearance === null);
  const keyBlocksCreation = isCheckingKey || keyLayer === "none" || editingKey;

  function showCardFace(nextMode: "default" | "custom") {
    if (visibleCardModeRef.current === nextMode) return;
    visibleCardModeRef.current = nextMode;
    flushSync(() => setVisibleCardMode(nextMode));
  }

  function handleCardModeUpdate(latest: { rotateY?: string | number }) {
    if (shouldReduceMotion) return;
    const rotateY =
      typeof latest.rotateY === "number"
        ? latest.rotateY
        : Number.parseFloat(latest.rotateY ?? "0");
    if (!Number.isFinite(rotateY)) return;
    showCardFace(rotateY >= 90 ? "custom" : "default");
  }

  function transitionCardMode(nextMode: "default" | "custom") {
    setMode(nextMode);

    if (shouldReduceMotion) {
      visibleCardModeRef.current = nextMode;
      setVisibleCardMode(nextMode);
      void cardModeControls.set({
        rotateY: 0,
        scale: nextMode === "custom" ? 0.8 : 1,
      });
      return;
    }

    void cardModeControls.start(
      {
        rotateY: nextMode === "custom" ? 180 : 0,
        scale: nextMode === "custom" ? 0.8 : 1,
      },
      CARD_MODE_TRANSITION,
    );
  }

  function enterCustomCardMode() {
    setCustomJobId(null);
    setCustomDescription("");
    setAvatarPreparationError(null);

    if (!activeCustomCard) {
      transitionCardMode("custom");
      return;
    }

    setMode("custom");
    visibleCardModeRef.current = "custom";
    setVisibleCardMode("custom");
    if (shouldReduceMotion) {
      void cardModeControls.set({ rotateY: 0, scale: 0.8 });
      return;
    }
    void cardModeControls.start(
      { rotateY: 180, scale: 0.8 },
      CARD_MODE_TRANSITION,
    );
  }

  async function beginCustomCard() {
    if (isCreatingCustomCard || customDescription.trim().length === 0) return;
    setAvatarPreparationError(null);
    setIsPreparingAvatar(true);
    try {
      const avatarDataUrl =
        await rasterizeInlineSvgAvatarForCard(agentAvatarUrl);
      setCustomJobId(
        startCardMint({
          agentId,
          agentName,
          avatarDataUrl,
          memoryLevel: hasLinkedAgent ? memoryLevel : "none",
          referenceCardPngBase64: activeCustomCard?.cardPngBase64 ?? undefined,
          styleNotes: customDescription.trim(),
        }),
      );
    } catch {
      setAvatarPreparationError(
        "This avatar couldn't be prepared for card creation. Try another avatar and create again.",
      );
    } finally {
      setIsPreparingAvatar(false);
    }
  }

  function cancelCustomCard() {
    setCustomJobId(null);
    setKeyDraft("");
    setEditingKey(false);
    setAvatarPreparationError(null);
    if (activeCustomCard) {
      setMode("default");
      visibleCardModeRef.current = "custom";
      setVisibleCardMode("custom");
      if (shouldReduceMotion) {
        void cardModeControls.set({ rotateY: 0, scale: 1 });
      } else {
        void cardModeControls.start(
          { rotateY: 180, scale: 1 },
          CARD_MODE_TRANSITION,
        );
      }
      return;
    }
    transitionCardMode("default");
  }

  async function exportCurrentCard() {
    if (activeCustomCard) {
      saveCustomCardMutation.mutate();
      return;
    }
    if (format === "json") {
      onSaveFile(memoryLevel, format);
      return;
    }

    setIsPreparingExport(true);
    try {
      const cardPngDataUrl = await renderAgentTradingCardPng({
        accentColor: defaultCardAppearance?.accentColor,
        agentAvatarUrl,
        agentId,
        agentName,
        avatarPngDataUrl: defaultCardAppearance?.avatarPngDataUrl,
        format,
        memoryLevel,
        templateUrl: tradingCardTemplateUrl,
      });
      onSaveFile(memoryLevel, format, cardPngDataUrl);
    } catch {
      toast.error("Couldn't prepare the card image for export.");
    } finally {
      setIsPreparingExport(false);
    }
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        aria-describedby={undefined}
        className="max-w-lg"
        showCloseButton={false}
        surface="none"
      >
        <div
          className="relative overflow-hidden rounded-2xl bg-background shadow-2xl"
          data-mode-size="stable"
          data-resize-motion={shouldReduceMotion ? "reduced" : "enabled"}
          data-testid="agent-snapshot-export-dialog"
          ref={setModalSurfaceElement}
        >
          <div
            className="relative grid h-full grid-rows-[auto_minmax(0,1fr)] gap-4 p-6"
            ref={setModalContentsElement}
          >
            <DialogClose
              className="absolute right-4 top-4 z-10 flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground transition-colors duration-150 ease-out hover:bg-accent hover:text-accent-foreground focus:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
              disabled={isPending}
            >
              <X className="h-4 w-4" />
              <span className="sr-only">Close</span>
            </DialogClose>
            <DialogHeader className="space-y-0">
              <DialogTitle className="truncate pr-10 text-left">
                {mode === "custom"
                  ? editingKey
                    ? "Update OpenAI key"
                    : "Create custom card"
                  : `Export ${agentName}`}
              </DialogTitle>
            </DialogHeader>

            <div
              className={`flex min-h-0 flex-col pt-2 ${compactForInlineDetails ? "gap-2" : "gap-6"}`}
            >
              <div
                className={`agent-card-mode-perspective ${compactForInlineDetails ? "py-0" : "py-2"}`}
                data-card-mode={mode}
                data-testid="agent-card-display-area"
              >
                <motion.div
                  animate={cardModeControls}
                  className="agent-card-mode-flip"
                  data-testid="agent-card-mode-flip"
                  initial={{ rotateY: 0, scale: 1 }}
                  onUpdate={handleCardModeUpdate}
                >
                  <div
                    aria-hidden={visibleCardMode === "custom"}
                    className="agent-card-mode-face agent-card-mode-face--front"
                    data-active={visibleCardMode === "default"}
                    data-testid="agent-card-default-face"
                  >
                    <AgentTradingCardPreview
                      agentAvatarUrl={agentAvatarUrl}
                      agentId={agentId}
                      agentName={agentName}
                      format={format}
                      memoryLevel={memoryLevel}
                      onAppearanceChange={setDefaultCardAppearance}
                    />
                  </div>

                  <div
                    aria-hidden={visibleCardMode === "default"}
                    className="agent-card-mode-face agent-card-mode-face--back"
                    data-active={visibleCardMode === "custom"}
                    data-testid="agent-card-custom-face"
                  >
                    <div className="grid">
                      <AnimatePresence initial={false} mode="sync">
                        {customCardImageUrl ? (
                          <motion.div
                            animate={{ opacity: 1 }}
                            className="col-start-1 row-start-1"
                            exit={{ opacity: 0 }}
                            initial={{ opacity: 0 }}
                            key={`custom-card-${customJobId ?? customCardRevealKey}`}
                            transition={cardContentTransition}
                          >
                            <AgentTradingCardPreview
                              agentAvatarUrl={agentAvatarUrl}
                              agentId={agentId}
                              agentName={agentName}
                              cardImageUrl={customCardImageUrl}
                              format="png"
                              memoryLevel={
                                displayedCustomCard?.memoryLevel ?? memoryLevel
                              }
                            />
                          </motion.div>
                        ) : (
                          <motion.div
                            animate={{ opacity: 1 }}
                            className="col-start-1 row-start-1"
                            exit={{ opacity: 0 }}
                            initial={{ opacity: 0 }}
                            key="custom-card-draft"
                            transition={cardContentTransition}
                          >
                            <AgentCustomCardDraftPreview
                              isCreating={isCreatingCustomCard}
                            />
                          </motion.div>
                        )}
                      </AnimatePresence>
                    </div>
                  </div>
                </motion.div>
              </div>

              <div className="grid min-h-0">
                <AnimatePresence initial={false} mode="sync">
                  {mode === "default" ? (
                    <motion.div
                      animate={{ opacity: 1, y: 0 }}
                      className={`col-start-1 row-start-1 flex min-w-0 flex-col ${showMemoryWarning ? "gap-2" : "gap-4"}`}
                      data-testid="agent-snapshot-export-settings"
                      exit={{ opacity: 0, y: 6 }}
                      initial={{ opacity: 0, y: 6 }}
                      key="default-settings"
                      transition={cardContentTransition}
                    >
                      <div className="space-y-2">
                        <div
                          className="mx-auto flex min-h-12 w-[calc(75%+2.25rem)] max-w-full items-center justify-between gap-4 px-1 py-2"
                          data-testid="agent-snapshot-memory-row"
                        >
                          <span className="flex min-w-0 items-center gap-2 text-sm font-medium">
                            <Brain className="h-4 w-4 shrink-0 text-muted-foreground" />
                            Memories
                          </span>
                          {hasLinkedAgent ? (
                            <SnapshotOptionMenu
                              ariaLabel="Memories"
                              className="font-medium text-foreground"
                              disabled={isPending || activeCustomCard !== null}
                              onValueChange={(value) =>
                                setMemoryLevel(value as SnapshotMemoryLevel)
                              }
                              options={MEMORY_LEVELS}
                              testId="agent-snapshot-memory-trigger"
                              value={memoryLevel}
                            />
                          ) : (
                            <span
                              className="inline-flex h-8 w-auto items-center justify-end px-2 text-sm font-medium"
                              data-testid="agent-snapshot-memory-value"
                            >
                              Agent only
                            </span>
                          )}
                        </div>

                        <div
                          className="mx-auto flex min-h-12 w-[calc(75%+2.25rem)] max-w-full items-center justify-between gap-4 px-1 py-2"
                          data-testid="agent-snapshot-format-row"
                        >
                          <span className="flex min-w-0 items-center gap-2 text-sm font-medium">
                            <FileType2 className="h-4 w-4 shrink-0 text-muted-foreground" />
                            File format
                          </span>
                          <SnapshotOptionMenu
                            ariaLabel="File format"
                            className="font-medium text-foreground"
                            disabled={isPending || activeCustomCard !== null}
                            onValueChange={(value) =>
                              setFormat(value as SnapshotFormat)
                            }
                            options={FORMAT_OPTIONS}
                            testId="agent-snapshot-format-trigger"
                            value={format}
                          />
                        </div>
                      </div>

                      <AnimatePresence initial={false}>
                        {showMemoryWarning ? (
                          <motion.div
                            animate={{ opacity: 1, y: 0 }}
                            data-testid="agent-snapshot-memory-warning-motion"
                            exit={{ opacity: 0, y: 4 }}
                            initial={{ opacity: 0, y: 4 }}
                            key="agent-snapshot-memory-warning"
                            transition={cardContentTransition}
                          >
                            <div
                              className="flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-700 dark:text-amber-400"
                              data-testid="agent-snapshot-memory-warning"
                            >
                              <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
                              <p>
                                Memory is stored as <strong>plaintext</strong>{" "}
                                in exported files. Only share those with people
                                you trust.
                              </p>
                            </div>
                          </motion.div>
                        ) : null}
                      </AnimatePresence>

                      <div
                        className="mx-auto flex w-[calc(75%+2.25rem)] max-w-full flex-col items-center gap-2 pt-2"
                        data-testid="agent-snapshot-export-footer"
                      >
                        <Button
                          className="w-full justify-center"
                          data-testid="agent-snapshot-export-confirm"
                          disabled={isPending}
                          onClick={() => void exportCurrentCard()}
                          type="button"
                        >
                          {isPreparingExport ? (
                            <Loader2 className="h-4 w-4 animate-spin" />
                          ) : (
                            <Download className="h-4 w-4" />
                          )}
                          {isPreparingExport ? "Preparing…" : "Export"}
                        </Button>
                        <Button
                          className="w-full justify-center"
                          data-testid="agent-custom-card-open"
                          disabled={isPending}
                          onClick={enterCustomCardMode}
                          type="button"
                          variant="outline"
                        >
                          <Sparkles className="h-4 w-4" />
                          Create custom card
                        </Button>
                      </div>
                    </motion.div>
                  ) : (
                    <motion.div
                      animate={{ opacity: 1, y: 0 }}
                      className="col-start-1 row-start-1 flex min-w-0 flex-col gap-4"
                      data-testid="agent-custom-card-settings"
                      exit={{ opacity: 0, y: 6 }}
                      initial={{ opacity: 0, y: 6 }}
                      key="custom-settings"
                      transition={cardContentTransition}
                    >
                      {isCreatingCustomCard ? (
                        <div
                          aria-live="polite"
                          className="mx-auto w-[calc(75%+2.25rem)] max-w-full space-y-2 pt-2"
                          data-testid="agent-custom-card-progress"
                        >
                          <div className="flex items-center justify-between gap-3 text-sm">
                            <span className="font-medium">Creating card…</span>
                            <span className="text-xs text-muted-foreground">
                              This can take a few minutes
                            </span>
                          </div>
                          <div
                            aria-label="Creating custom card"
                            aria-valuetext="Working"
                            className="agent-custom-card-progress__track"
                            role="progressbar"
                          >
                            <span className="agent-custom-card-progress__indicator" />
                          </div>
                        </div>
                      ) : editingKey ? (
                        <div
                          className="space-y-3 rounded-md border border-border p-3"
                          data-testid="agent-custom-card-key-setup"
                        >
                          <div className="space-y-1.5">
                            <span className="flex items-center gap-1.5 text-sm font-medium">
                              <KeyRound className="h-3.5 w-3.5" />
                              {keyLayer === "none"
                                ? "Add OpenAI API key"
                                : "Update OpenAI API key"}
                            </span>
                            {needsOpenAIKeyReplacement ? (
                              <p
                                aria-live="polite"
                                className="text-sm text-destructive"
                                data-testid="agent-custom-card-invalid-key"
                              >
                                That OpenAI key is invalid or expired. Add a
                                replacement to continue creating this card.
                              </p>
                            ) : null}
                            <p className="text-xs text-muted-foreground">
                              Creating custom card art uses the OpenAI API and
                              may cost money, billed by OpenAI. This key is
                              stored securely for custom cards only and does not
                              change Agent Defaults.
                            </p>
                            <Button
                              className="h-auto px-0 text-xs"
                              data-testid="agent-custom-card-key-link"
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
                            <Input
                              autoFocus
                              data-testid="agent-custom-card-key-input"
                              disabled={saveKeyMutation.isPending}
                              onChange={(event) =>
                                setKeyDraft(event.target.value)
                              }
                              placeholder="sk-…"
                              type="password"
                              value={keyDraft}
                            />
                          </div>
                          <div className="flex items-center justify-end gap-2">
                            <Button
                              data-testid="agent-custom-card-key-cancel"
                              disabled={saveKeyMutation.isPending}
                              onClick={() => {
                                setKeyDraft("");
                                setEditingKey(false);
                              }}
                              size="sm"
                              type="button"
                              variant="ghost"
                            >
                              Cancel
                            </Button>
                            <Button
                              data-testid="agent-custom-card-key-save"
                              disabled={
                                saveKeyMutation.isPending ||
                                keyDraft.trim().length === 0
                              }
                              onClick={() =>
                                saveKeyMutation.mutate(keyDraft.trim())
                              }
                              size="sm"
                              type="button"
                            >
                              {saveKeyMutation.isPending ? (
                                <Loader2 className="h-4 w-4 animate-spin" />
                              ) : (
                                <KeyRound className="h-4 w-4" />
                              )}
                              {saveKeyMutation.isPending
                                ? "Saving…"
                                : "Save key & continue"}
                            </Button>
                          </div>
                        </div>
                      ) : (
                        <>
                          <Textarea
                            autoFocus
                            className="mx-auto h-[6.5rem] w-[calc(75%+2.25rem)] max-w-full resize-none border-0 bg-muted/60 shadow-none focus-visible:ring-0"
                            data-testid="agent-custom-card-description"
                            disabled={isCreatingCustomCard}
                            onChange={(event) =>
                              setCustomDescription(event.target.value)
                            }
                            placeholder={
                              activeCustomCard
                                ? "Describe a follow-up"
                                : "Describe the card you want to create"
                            }
                            rows={4}
                            value={customDescription}
                          />

                          {isCheckingKey ? (
                            <div
                              className="flex items-center gap-1.5 text-xs text-muted-foreground"
                              data-testid="agent-custom-card-key-checking"
                            >
                              <Loader2 className="h-3 w-3 animate-spin" />
                              Checking for an OpenAI key…
                            </div>
                          ) : keyLayer === "none" ? (
                            <div
                              className="flex items-center justify-between gap-3 rounded-md border border-border p-3"
                              data-testid="agent-custom-card-key-required"
                            >
                              <span className="flex items-center gap-1.5 text-sm font-medium">
                                <KeyRound className="h-3.5 w-3.5" />
                                OpenAI key required
                              </span>
                              <Button
                                data-testid="agent-custom-card-key-add"
                                onClick={() => setEditingKey(true)}
                                size="sm"
                                type="button"
                                variant="outline"
                              >
                                Add OpenAI key
                              </Button>
                            </div>
                          ) : null}

                          {avatarPreparationError ||
                          (customJob?.phase === "error" && customJob.error) ? (
                            <p
                              aria-live="polite"
                              className="text-sm text-destructive"
                              data-testid="agent-custom-card-error"
                            >
                              {avatarPreparationError ?? customJob?.error}
                            </p>
                          ) : null}

                          <div className="mx-auto flex w-[calc(75%+2.25rem)] max-w-full flex-col items-center gap-2 pt-2">
                            <Button
                              className="w-full justify-center"
                              data-testid="agent-custom-card-create"
                              disabled={
                                isCreatingCustomCard ||
                                keyBlocksCreation ||
                                customDescription.trim().length === 0
                              }
                              onClick={beginCustomCard}
                              type="button"
                            >
                              {isCreatingCustomCard ? (
                                <Loader2 className="h-4 w-4 animate-spin" />
                              ) : (
                                <Sparkles className="h-4 w-4" />
                              )}
                              {isCreatingCustomCard ? "Creating…" : "Create"}
                            </Button>
                            <Button
                              className="w-full justify-center"
                              data-testid="agent-custom-card-back"
                              disabled={isPending}
                              onClick={cancelCustomCard}
                              type="button"
                              variant="outline"
                            >
                              Back
                            </Button>
                          </div>
                        </>
                      )}
                    </motion.div>
                  )}
                </AnimatePresence>
              </div>
            </div>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
