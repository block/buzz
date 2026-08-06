import * as React from "react";
import { Download, Lock, RefreshCw, Send, Sparkles } from "lucide-react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { toast } from "sonner";

import {
  closeCardViewer,
  openCardViewer,
  setCardGalleryOpen,
  startCardMint,
  useCardGalleryOpen,
  useCardViewerState,
  type CardMintInput,
} from "@/features/agents/cardMintStore";
import { agentCardGalleryViewState } from "@/features/agents/lib/agentCardGalleryState";
import {
  useOpenDmMutation,
  useUpsertCachedChannel,
} from "@/features/channels/hooks";
import {
  listAgentCards,
  loadAgentCard,
  saveAgentCard,
  type ArchivedAgentCard,
  type MintedAgentCard,
} from "@/shared/api/tauriPersonas";
import type { UserSearchResult } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  MediaContextMenu,
  type MediaContextMenuPosition,
  useDismissMediaContextMenu,
} from "@/shared/ui/markdown/MediaContextMenu";

import { PersonaShareRecipients } from "./PersonaShareRecipients";
import { useSnapshotSendController } from "./useSnapshotSendController";

/** Decode base64 card bytes into the number[] shape the upload path expects. */
function cardBytesFromBase64(b64: string): number[] {
  return Array.from(atob(b64), (char) => char.charCodeAt(0));
}

/**
 * Global card surfaces, mounted once in `AppShell` next to the other global
 * dialogs and driven by `cardMintStore`: the finished-card viewer plus the
 * previous-mints gallery.
 */
export function AgentCardDialogs() {
  return (
    <>
      <AgentCardViewerDialog />
      <AgentCardGalleryDialog />
    </>
  );
}

/**
 * Global viewer for a finished agent trading card — the post-mint half of the
 * old blocking mint dialog, now openable from anywhere (completion toast,
 * composer "card ready" chip, or the previous-cards gallery). Mounted once in
 * `AppShell` next to the other global dialogs and driven by `cardMintStore`.
 *
 * The card image carries the same right-click download menu inline channel
 * images have (`MediaContextMenu`), but routed through `save_agent_card`:
 * `download_image` only accepts relay `/media/` URLs, and this card exists
 * only as local base64.
 */
export function AgentCardViewerDialog() {
  const state = useCardViewerState();

  // Keyed remount resets recipients/menu state when a different card opens.
  if (!state) return null;
  return (
    <AgentCardViewerContent
      key={state.viewerSeq}
      agentName={state.agentName}
      card={state.card}
      remint={state.remint}
    />
  );
}

function AgentCardViewerContent({
  agentName,
  card,
  remint,
}: {
  agentName: string;
  card: MintedAgentCard;
  remint: CardMintInput | null;
}) {
  const [recipients, setRecipients] = React.useState<UserSearchResult[]>([]);
  const [menu, setMenu] = React.useState<MediaContextMenuPosition | null>(null);
  const closeMenu = React.useCallback(() => setMenu(null), []);
  useDismissMediaContextMenu(Boolean(menu), closeMenu);

  const sendController = useSnapshotSendController(true);
  const openDmMutation = useOpenDmMutation();
  const upsertCachedChannel = useUpsertCachedChannel();

  const saveMutation = useMutation({
    mutationFn: () => saveAgentCard(card.cardPngBase64, card.fileName),
    onSuccess: (saved) => {
      if (saved) {
        toast.success(
          `Saved ${agentName}'s card. Share it — the card IS the agent.`,
        );
      }
    },
    onError: (error: Error) => toast.error(error.message),
  });

  const isSending =
    sendController.state.phase !== "idle" &&
    sendController.state.phase !== "done" &&
    sendController.state.phase !== "error";
  const isBusy = saveMutation.isPending || isSending;

  async function sendToRecipients() {
    if (recipients.length === 0) return;
    const sent = await sendController.beginSend(
      async () => ({
        fileBytes: cardBytesFromBase64(card.cardPngBase64),
        fileName: card.fileName,
      }),
      async () => {
        const directMessage = await openDmMutation.mutateAsync({
          pubkeys: recipients.map((recipient) => recipient.pubkey),
        });
        await upsertCachedChannel(directMessage);
        return directMessage.id;
      },
      agentName,
    );
    if (sent) {
      toast.success(`Sent ${agentName}'s card.`);
      closeCardViewer();
    } else if (sent === false) {
      toast.error("Couldn’t send the card. Try again.");
    }
  }

  return (
    <Dialog onOpenChange={(open) => !open && closeCardViewer()} open>
      <DialogContent
        className="max-w-md"
        data-testid="agent-card-viewer-dialog"
      >
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Sparkles className="h-4 w-4" />
            {`${agentName}'s card`}
          </DialogTitle>
          <DialogDescription>
            {card.locked
              ? "This card is locked: only you and the agent can import it. Anyone else sees just the image."
              : card.memoryLevel === "none"
                ? "The card carries the agent — anyone who imports this PNG gets a working copy (config only, fresh identity, no memories)."
                : `The card carries the agent — anyone who imports this PNG gets a working copy (fresh identity) including its ${card.memoryLevel === "core" ? "core memory" : "memories"}, stored as plaintext.`}
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-4">
          <img
            alt={`${agentName} trading card`}
            className="mx-auto max-h-[28rem] rounded-lg border shadow-lg"
            data-testid="agent-card-preview"
            onContextMenu={(event) => {
              event.preventDefault();
              setMenu({ x: event.clientX, y: event.clientY });
            }}
            src={`data:image/png;base64,${card.cardPngBase64}`}
          />
          {menu ? (
            <MediaContextMenu
              dataAttributes={["data-agent-card-context-menu"]}
              items={[
                {
                  label: "Download card",
                  onSelect: () => {
                    closeMenu();
                    saveMutation.mutate();
                  },
                },
              ]}
              position={menu}
            />
          ) : null}
          <PersonaShareRecipients
            disabled={isBusy || !sendController.isDmSafetyReady}
            excludedPubkeys={
              sendController.relaySelfPubkey
                ? [sendController.relaySelfPubkey]
                : []
            }
            onSelectionChange={setRecipients}
            open
            selectedUsers={recipients}
            testIdPrefix="agent-card-share"
          />
          <div className="flex justify-end gap-2">
            {remint ? (
              <Button
                disabled={isBusy}
                data-testid="agent-card-reroll"
                onClick={() => {
                  // Reroll = a fresh background mint with the same input; the
                  // viewer closes and the composer chip takes over.
                  startCardMint(remint);
                  closeCardViewer();
                }}
                variant="outline"
              >
                <RefreshCw className="mr-2 h-4 w-4" />
                Reroll
              </Button>
            ) : null}
            <Button
              disabled={isBusy}
              onClick={() => saveMutation.mutate()}
              data-testid="agent-card-save"
              variant={recipients.length > 0 ? "outline" : "default"}
            >
              <Download className="mr-2 h-4 w-4" />
              Save card
            </Button>
            {recipients.length > 0 ? (
              <Button
                disabled={isBusy}
                onClick={() => void sendToRecipients()}
                data-testid="agent-card-send"
              >
                <Send className="mr-2 h-4 w-4" />
                {isSending ? "Sending…" : "Send"}
              </Button>
            ) : null}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

/**
 * Gallery of all previously minted cards, newest first. Every successful mint
 * is archived to the app data dir by Rust; clicking a tile loads the full PNG
 * and opens it in the viewer (save/share, no reroll — the original style
 * notes are not retained).
 */
export function AgentCardGalleryDialog() {
  const open = useCardGalleryOpen();
  if (!open) return null;
  return <AgentCardGalleryContent />;
}

function AgentCardGalleryContent() {
  const cardsQuery = useQuery({
    queryKey: ["agentCardArchive"],
    queryFn: listAgentCards,
  });

  const openMutation = useMutation({
    mutationFn: async (entry: ArchivedAgentCard) => {
      const cardPngBase64 = await loadAgentCard(entry.storedFileName);
      return { entry, cardPngBase64 };
    },
    onSuccess: ({ entry, cardPngBase64 }) => {
      setCardGalleryOpen(false);
      openCardViewer({
        agentName: entry.agentName,
        card: {
          cardPngBase64,
          fileName: entry.fileName,
          designerNotes: entry.designerNotes,
          locked: entry.locked,
          memoryLevel: entry.memoryLevel,
        },
        remint: null,
      });
    },
    onError: (error: Error) => toast.error(error.message),
  });

  const cards = cardsQuery.data ?? [];
  const viewState = agentCardGalleryViewState(cardsQuery);

  return (
    <Dialog onOpenChange={(open) => setCardGalleryOpen(open)} open>
      <DialogContent
        className="max-w-lg"
        data-testid="agent-card-gallery-dialog"
      >
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Sparkles className="h-4 w-4" />
            Minted cards
          </DialogTitle>
          <DialogDescription>
            Every card you’ve minted, newest first. Click one to view, save, or
            share it.
          </DialogDescription>
        </DialogHeader>
        {viewState.kind === "loading" ? (
          <p className="py-8 text-center text-sm text-muted-foreground">
            Loading cards…
          </p>
        ) : viewState.kind === "error" ? (
          <div
            className="flex flex-col items-center gap-3 py-8"
            data-testid="agent-card-gallery-error"
            role="alert"
          >
            <p className="text-center text-sm text-destructive">
              Couldn’t load your minted cards: {viewState.message}
            </p>
            <Button
              disabled={cardsQuery.isFetching}
              onClick={() => void cardsQuery.refetch()}
              size="sm"
              variant="outline"
            >
              <RefreshCw className="mr-2 h-4 w-4" />
              {cardsQuery.isFetching ? "Retrying…" : "Retry"}
            </Button>
          </div>
        ) : viewState.kind === "empty" ? (
          <p className="py-8 text-center text-sm text-muted-foreground">
            No cards yet — mint one from an agent’s profile.
          </p>
        ) : (
          <div className="grid max-h-[60vh] grid-cols-3 gap-3 overflow-y-auto pr-1">
            {cards.map((entry) => (
              <button
                className="group flex flex-col gap-1.5 text-left"
                data-testid="agent-card-gallery-item"
                disabled={openMutation.isPending}
                key={entry.storedFileName}
                onClick={() => openMutation.mutate(entry)}
                type="button"
              >
                {entry.thumbJpegBase64 ? (
                  <img
                    alt={`${entry.agentName} trading card`}
                    className="aspect-2/3 w-full rounded-lg border object-cover shadow-sm transition-transform group-hover:scale-[1.02]"
                    src={`data:image/jpeg;base64,${entry.thumbJpegBase64}`}
                  />
                ) : (
                  <div className="flex aspect-2/3 w-full items-center justify-center rounded-lg border bg-muted/40">
                    <Sparkles className="h-6 w-6 text-muted-foreground" />
                  </div>
                )}
                <span className="flex items-center gap-1 truncate text-xs text-muted-foreground">
                  {entry.locked ? <Lock className="h-3 w-3 shrink-0" /> : null}
                  <span className="truncate">{entry.agentName}</span>
                </span>
              </button>
            ))}
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
