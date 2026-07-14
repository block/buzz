import * as React from "react";
import { AlertCircle, Check, Search, Send } from "lucide-react";
import { useMutation } from "@tanstack/react-query";

import type { AgentTeam } from "@/shared/api/types";
import type { SnapshotMemoryLevel } from "@/shared/api/tauriTeams";
import { encodeTeamSnapshotForSend } from "@/shared/api/tauriTeams";
import { buildTeamSnapshotSendArgs } from "./teamSnapshotImport.lib";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Separator } from "@/shared/ui/separator";
import { useTimeoutState } from "@/features/moderation/lib/timeoutStore";
import {
  useSnapshotSendController,
  type SendPhase,
  type ResolvedChannel,
} from "./useSnapshotSendController";
import { MemoryGateStep } from "./AgentSnapshotSendDialog";

// ── Types ─────────────────────────────────────────────────────────────────────

type TeamSnapshotSendDialogProps = {
  open: boolean;
  team: AgentTeam;
  memoryLevel: SnapshotMemoryLevel;
  onOpenChange: (open: boolean) => void;
  /** Called when the snapshot was successfully sent. */
  onSent: () => void;
};

/** UI step within this dialog. */
type DialogStep =
  | "pick" // destination picker
  | "memgate" // destination-scoped memory confirmation (memory-bearing only)
  | "progress" // uploading / sending
  | "done" // success
  | "error"; // unrecoverable error

// ── Component ─────────────────────────────────────────────────────────────────

export function TeamSnapshotSendDialog({
  open,
  team,
  memoryLevel,
  onOpenChange,
  onSent,
}: TeamSnapshotSendDialogProps) {
  const controller = useSnapshotSendController();
  const encodeMutation = useMutation({
    mutationFn: ({
      id,
      memoryLevel,
      format,
    }: ReturnType<typeof buildTeamSnapshotSendArgs>) =>
      encodeTeamSnapshotForSend(id, memoryLevel, format),
  });
  const timeoutState = useTimeoutState();

  const [step, setStep] = React.useState<DialogStep>("pick");
  const [selectedChannel, setSelectedChannel] =
    React.useState<ResolvedChannel | null>(null);
  const [search, setSearch] = React.useState("");

  const hasMemory = memoryLevel !== "none";
  const isInProgress =
    controller.state.phase === "preparing" ||
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

  // Reconcile selectedChannel when sendableChannels updates.
  React.useEffect(() => {
    if (selectedChannel === null) return;
    const current = controller.sendableChannels.find(
      (ch) => ch.id === selectedChannel.id,
    );
    if (!current) {
      setSelectedChannel(null);
    } else if (current !== selectedChannel) {
      setSelectedChannel(current);
    }
  }, [controller.sendableChannels, selectedChannel]);

  const filteredChannels = React.useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return controller.sendableChannels;
    return controller.sendableChannels.filter(
      (ch) =>
        ch.displayLabel.toLowerCase().includes(q) ||
        ch.name.toLowerCase().includes(q),
    );
  }, [controller.sendableChannels, search]);

  // ── Step handlers ──────────────────────────────────────────────────────────

  function handlePickConfirm() {
    if (!selectedChannel) return;
    if (hasMemory) {
      setStep("memgate");
    } else {
      void handleSend(selectedChannel);
    }
  }

  async function handleSend(destination: ResolvedChannel) {
    if (timeoutState.active) {
      controller.setErrorState(
        "You are currently timed out and cannot send messages.",
      );
      setStep("error");
      return;
    }

    const stillSendable = controller.sendableChannels.some(
      (ch) => ch.id === destination.id,
    );
    if (!stillSendable) {
      controller.setErrorState(
        "The selected destination is no longer available. Please pick another.",
      );
      setStep("error");
      return;
    }

    setStep("progress");
    await controller.beginSend(
      async () =>
        encodeMutation.mutateAsync(
          buildTeamSnapshotSendArgs(team.id, memoryLevel),
        ),
      destination.id,
    );
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

  const selectedLabel = React.useMemo(() => {
    if (!selectedChannel) return null;
    return selectedChannel.channelType === "dm"
      ? `the DM with ${selectedChannel.displayLabel}`
      : `#${selectedChannel.displayLabel}`;
  }, [selectedChannel]);

  // ── Render ─────────────────────────────────────────────────────────────────

  return (
    <Dialog onOpenChange={handleClose} open={open}>
      <DialogContent
        aria-describedby={undefined}
        className="max-w-md"
        data-testid="team-snapshot-send-dialog"
        showCloseButton={false}
      >
        <DialogHeader className="space-y-0">
          <div className="flex items-center justify-between gap-4">
            <DialogTitle>
              {step === "done"
                ? "Snapshot sent"
                : step === "memgate"
                  ? "Confirm memory share"
                  : "Send team snapshot in Buzz"}
            </DialogTitle>
            {step !== "progress" && step !== "done" && step !== "error" ? (
              <div className="flex items-center gap-2">
                {step === "pick" ? (
                  <Button
                    data-testid="team-snapshot-send-confirm"
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
                    data-testid="team-snapshot-send-memgate-confirm"
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
            team={team}
            channels={filteredChannels}
            isLoadingChannels={controller.isLoadingChannels}
            selectedChannel={selectedChannel}
            search={search}
            onSearchChange={setSearch}
            onSelectChannel={setSelectedChannel}
          />
        ) : step === "memgate" && selectedChannel !== null ? (
          <MemoryGateStep
            destinationLabel={selectedLabel ?? `#${selectedChannel.name}`}
            memoryLevel={memoryLevel}
          />
        ) : step === "progress" ? (
          <ProgressStep phase={controller.state.phase} />
        ) : step === "done" && selectedChannel !== null ? (
          <DoneStep
            destinationLabel={selectedLabel ?? `#${selectedChannel.name}`}
          />
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
  team,
  channels,
  isLoadingChannels,
  selectedChannel,
  search,
  onSearchChange,
  onSelectChannel,
}: {
  team: AgentTeam;
  channels: ResolvedChannel[];
  isLoadingChannels: boolean;
  selectedChannel: ResolvedChannel | null;
  search: string;
  onSearchChange: (s: string) => void;
  onSelectChannel: (ch: ResolvedChannel) => void;
}) {
  return (
    <div className="space-y-4 py-1">
      <p className="text-sm text-muted-foreground">
        Send <span className="font-medium text-foreground">{team.name}</span> as
        a team snapshot attachment to a channel or DM. Recipients receive the
        snapshot file and can import it locally.
      </p>

      {/* Search */}
      <div className="relative">
        <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
        <input
          className="w-full rounded-md border border-input bg-background py-2 pl-8 pr-3 text-sm placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
          data-testid="team-snapshot-send-search"
          placeholder="Search channels and DMs…"
          type="text"
          value={search}
          onChange={(e) => onSearchChange(e.target.value)}
        />
      </div>

      {/* Channel list */}
      <div
        className="max-h-52 overflow-y-auto rounded-md border border-border"
        data-testid="team-snapshot-send-channel-list"
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
              data-testid={`team-snapshot-send-channel-${ch.id}`}
              key={ch.id}
              type="button"
              onClick={() => onSelectChannel(ch)}
            >
              <span className="text-muted-foreground text-xs w-5 shrink-0">
                {ch.channelType === "dm" ? "DM" : "#"}
              </span>
              <span className="truncate">{ch.displayLabel}</span>
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

// ── Progress step ─────────────────────────────────────────────────────────────

function ProgressStep({ phase }: { phase: SendPhase }) {
  const label =
    phase === "preparing"
      ? "Preparing snapshot…"
      : phase === "uploading"
        ? "Uploading snapshot…"
        : "Sending message…";
  return (
    <div
      className="py-6 text-center text-sm text-muted-foreground"
      data-testid="team-snapshot-send-progress"
    >
      {label}
    </div>
  );
}

// ── Done step ─────────────────────────────────────────────────────────────────

function DoneStep({ destinationLabel }: { destinationLabel: string }) {
  return (
    <div
      className="space-y-2 py-2 text-sm"
      data-testid="team-snapshot-send-done"
    >
      <p>
        Team snapshot sent to{" "}
        <span className="font-medium">{destinationLabel}</span>. Recipients can
        download and import the snapshot file.
      </p>
    </div>
  );
}

// ── Error step ────────────────────────────────────────────────────────────────

function ErrorStep({ error }: { error: string }) {
  return (
    <div
      className="flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
      data-testid="team-snapshot-send-error"
    >
      <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
      <p>{error}</p>
    </div>
  );
}
