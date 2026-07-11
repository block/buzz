import * as React from "react";
import { AlertCircle, Check, Search, Send } from "lucide-react";

import type { AgentPersona } from "@/shared/api/types";
import type {
  SnapshotFormat,
  SnapshotMemoryLevel,
} from "@/shared/api/tauriPersonas";
import type { Channel } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Separator } from "@/shared/ui/separator";
import { useEncodeAgentSnapshotForSendMutation } from "@/features/agents/hooks";
import {
  useSnapshotSendController,
  type SendPhase,
} from "./useSnapshotSendController";

// ── Types ─────────────────────────────────────────────────────────────────────

type AgentSnapshotSendDialogProps = {
  open: boolean;
  persona: AgentPersona;
  linkedAgentPubkey: string | null;
  memoryLevel: SnapshotMemoryLevel;
  format: SnapshotFormat;
  onOpenChange: (open: boolean) => void;
  /** Called when the snapshot was successfully sent. */
  onSent: () => void;
};

// ── Sub-phases within the dialog ──────────────────────────────────────────────

/** UI step within this dialog. */
type DialogStep =
  | "pick" // destination picker
  | "memgate" // destination-scoped memory confirmation (memory-bearing only)
  | "progress" // uploading / sending
  | "done" // success
  | "error"; // unrecoverable error

// ── Component ─────────────────────────────────────────────────────────────────

export function AgentSnapshotSendDialog({
  open,
  persona,
  linkedAgentPubkey,
  memoryLevel,
  format,
  onOpenChange,
  onSent,
}: AgentSnapshotSendDialogProps) {
  const controller = useSnapshotSendController();
  const encodeMutation = useEncodeAgentSnapshotForSendMutation();

  const [step, setStep] = React.useState<DialogStep>("pick");
  const [selectedChannel, setSelectedChannel] = React.useState<Channel | null>(
    null,
  );
  const [search, setSearch] = React.useState("");

  const hasMemory = memoryLevel !== "none";
  const isInProgress =
    encodeMutation.isPending ||
    controller.state.phase === "uploading" ||
    controller.state.phase === "sending";

  // Reset all transient state when the dialog opens.
  // biome-ignore lint/correctness/useExhaustiveDependencies: controller.reset and encodeMutation.reset are stable function references; only `open` drives the reset
  React.useEffect(() => {
    if (open) {
      setStep("pick");
      setSelectedChannel(null);
      setSearch("");
      controller.reset();
      encodeMutation.reset();
    }
  }, [open]);

  // Mirror controller phase transitions to dialog step.
  React.useEffect(() => {
    if (controller.state.phase === "done") {
      setStep("done");
    } else if (controller.state.phase === "error") {
      setStep("error");
    }
  }, [controller.state.phase]);

  const filteredChannels = React.useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return controller.sendableChannels;
    return controller.sendableChannels.filter((ch) =>
      ch.name.toLowerCase().includes(q),
    );
  }, [controller.sendableChannels, search]);

  // ── Step handlers ──────────────────────────────────────────────────────────

  function handlePickConfirm() {
    if (!selectedChannel) return;
    if (hasMemory) {
      // Memory-bearing: require explicit destination-scoped confirmation first.
      setStep("memgate");
    } else {
      void handleSend(selectedChannel);
    }
  }

  async function handleSend(destination: Channel) {
    setStep("progress");
    try {
      const payload = await encodeMutation.mutateAsync({
        id: persona.id,
        memoryLevel,
        format,
        memorySourcePubkey: linkedAgentPubkey,
      });
      await controller.sendPayload(
        payload.fileBytes,
        payload.fileName,
        destination.id,
      );
    } catch (_err) {
      // Encode errors: set error step directly.
      setStep("error");
    }
  }

  function handleMemgateConfirm() {
    if (selectedChannel) {
      void handleSend(selectedChannel);
    }
  }

  function handleClose() {
    if (!isInProgress) {
      onOpenChange(false);
    }
  }

  function handleDoneClose() {
    onOpenChange(false);
    onSent();
  }

  // ── Render ─────────────────────────────────────────────────────────────────

  return (
    <Dialog onOpenChange={handleClose} open={open}>
      <DialogContent
        aria-describedby={undefined}
        className="max-w-md"
        data-testid="agent-snapshot-send-dialog"
        showCloseButton={false}
      >
        <DialogHeader className="space-y-0">
          <div className="flex items-center justify-between gap-4">
            <DialogTitle>
              {step === "done"
                ? "Snapshot sent"
                : step === "memgate"
                  ? "Confirm memory share"
                  : "Send snapshot in Buzz"}
            </DialogTitle>
            {step !== "progress" && step !== "done" && step !== "error" ? (
              <div className="flex items-center gap-2">
                {step === "pick" ? (
                  <Button
                    data-testid="agent-snapshot-send-confirm"
                    disabled={selectedChannel === null || isInProgress}
                    onClick={handlePickConfirm}
                    size="sm"
                    type="button"
                    variant="default"
                  >
                    <Send className="h-4 w-4" />
                    {hasMemory ? "Next" : "Send"}
                  </Button>
                ) : (
                  // memgate step
                  <Button
                    data-testid="agent-snapshot-send-memgate-confirm"
                    disabled={isInProgress}
                    onClick={handleMemgateConfirm}
                    size="sm"
                    type="button"
                    variant="default"
                  >
                    <Send className="h-4 w-4" />
                    Send anyway
                  </Button>
                )}
                <DialogClose asChild>
                  <Button
                    disabled={isInProgress}
                    size="sm"
                    type="button"
                    variant="ghost"
                  >
                    Cancel
                  </Button>
                </DialogClose>
              </div>
            ) : step === "done" ? (
              <Button
                onClick={handleDoneClose}
                size="sm"
                type="button"
                variant="ghost"
              >
                Close
              </Button>
            ) : step === "error" ? (
              <div className="flex items-center gap-2">
                <Button
                  onClick={() => {
                    controller.reset();
                    encodeMutation.reset();
                    setStep("pick");
                  }}
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  Try again
                </Button>
                <DialogClose asChild>
                  <Button size="sm" type="button" variant="ghost">
                    Close
                  </Button>
                </DialogClose>
              </div>
            ) : null}
          </div>
        </DialogHeader>

        <Separator />

        {step === "pick" ? (
          <PickStep
            persona={persona}
            channels={filteredChannels}
            isLoadingChannels={controller.isLoadingChannels}
            selectedChannel={selectedChannel}
            search={search}
            onSearchChange={setSearch}
            onSelectChannel={setSelectedChannel}
          />
        ) : step === "memgate" && selectedChannel !== null ? (
          <MemoryGateStep
            destination={selectedChannel}
            memoryLevel={memoryLevel}
          />
        ) : step === "progress" ? (
          <ProgressStep phase={controller.state.phase} />
        ) : step === "done" && selectedChannel !== null ? (
          <DoneStep destination={selectedChannel} />
        ) : (
          <ErrorStep
            error={
              controller.state.error ??
              (encodeMutation.error instanceof Error
                ? encodeMutation.error.message
                : "An unexpected error occurred.")
            }
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

// ── Pick step ─────────────────────────────────────────────────────────────────

function PickStep({
  persona,
  channels,
  isLoadingChannels,
  selectedChannel,
  search,
  onSearchChange,
  onSelectChannel,
}: {
  persona: AgentPersona;
  channels: Channel[];
  isLoadingChannels: boolean;
  selectedChannel: Channel | null;
  search: string;
  onSearchChange: (s: string) => void;
  onSelectChannel: (ch: Channel) => void;
}) {
  return (
    <div className="space-y-4 py-1">
      <p className="text-sm text-muted-foreground">
        Send{" "}
        <span className="font-medium text-foreground">
          {persona.displayName}
        </span>{" "}
        as a snapshot to a channel or DM. The recipient can import it directly
        from the message.
      </p>

      {/* Search */}
      <div className="relative">
        <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
        <input
          className="w-full rounded-md border border-input bg-background py-2 pl-8 pr-3 text-sm placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
          data-testid="agent-snapshot-send-search"
          placeholder="Search channels and DMs…"
          type="text"
          value={search}
          onChange={(e) => onSearchChange(e.target.value)}
        />
      </div>

      {/* Channel list */}
      <div
        className="max-h-52 overflow-y-auto rounded-md border border-border"
        data-testid="agent-snapshot-send-channel-list"
      >
        {isLoadingChannels ? (
          <p className="px-3 py-4 text-center text-sm text-muted-foreground">
            Loading…
          </p>
        ) : channels.length === 0 ? (
          <p className="px-3 py-4 text-center text-sm text-muted-foreground">
            {search
              ? "No matching destinations."
              : "No sendable destinations found."}
          </p>
        ) : (
          channels.map((ch) => (
            <button
              className={`flex w-full items-center gap-2 px-3 py-2 text-left text-sm transition-colors hover:bg-muted ${
                selectedChannel?.id === ch.id ? "bg-muted font-medium" : ""
              }`}
              data-testid={`agent-snapshot-send-channel-${ch.id}`}
              key={ch.id}
              type="button"
              onClick={() => onSelectChannel(ch)}
            >
              <span className="text-muted-foreground text-xs w-5 shrink-0">
                {ch.channelType === "dm" ? "DM" : "#"}
              </span>
              <span className="truncate">{ch.name}</span>
              {selectedChannel?.id === ch.id ? (
                <Check className="ml-auto h-4 w-4 shrink-0 text-primary" />
              ) : null}
            </button>
          ))
        )}
      </div>
    </div>
  );
}

// ── Memory gate step ──────────────────────────────────────────────────────────

export function MemoryGateStep({
  destination,
  memoryLevel,
}: {
  destination: Channel;
  memoryLevel: SnapshotMemoryLevel;
}) {
  const scope = memoryLevel === "core" ? "core memory" : "all memory";
  const destLabel =
    destination.channelType === "dm"
      ? `the DM with ${destination.name}`
      : `#${destination.name}`;

  return (
    <div className="space-y-4 py-1">
      <div
        className="flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-700 dark:text-amber-400"
        data-testid="agent-snapshot-send-memory-gate"
      >
        <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
        <div className="space-y-1.5">
          <p>
            This snapshot includes <strong>plaintext {scope}</strong>.
          </p>
          <ul className="list-disc space-y-1 pl-4 text-xs">
            <li>
              The memory will be delivered to <strong>{destLabel}</strong> and
              visible to everyone in that recipient surface.
            </li>
            <li>
              Anyone who obtains the uploaded media link will also be able to
              fetch the raw snapshot bytes.
            </li>
          </ul>
          <p>Only continue if you trust everyone who can see {destLabel}.</p>
        </div>
      </div>
    </div>
  );
}

// ── Progress step ─────────────────────────────────────────────────────────────

function ProgressStep({ phase }: { phase: SendPhase }) {
  const label =
    phase === "uploading" ? "Uploading snapshot…" : "Sending message…";
  return (
    <div
      className="py-6 text-center text-sm text-muted-foreground"
      data-testid="agent-snapshot-send-progress"
    >
      {label}
    </div>
  );
}

// ── Done step ─────────────────────────────────────────────────────────────────

function DoneStep({ destination }: { destination: Channel }) {
  const destLabel =
    destination.channelType === "dm"
      ? `the DM with ${destination.name}`
      : `#${destination.name}`;

  return (
    <div
      className="space-y-2 py-2 text-sm"
      data-testid="agent-snapshot-send-done"
    >
      <p>
        Snapshot sent to <span className="font-medium">{destLabel}</span>.
        Recipients can click the attachment to import the agent.
      </p>
    </div>
  );
}

// ── Error step ────────────────────────────────────────────────────────────────

function ErrorStep({ error }: { error: string }) {
  return (
    <div
      className="flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
      data-testid="agent-snapshot-send-error"
    >
      <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
      <p>{error}</p>
    </div>
  );
}
