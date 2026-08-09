import * as React from "react";

import {
  REMOTE_AGENT_MODEL_OPTIONS,
  REMOTE_AGENT_PRESETS,
  type RemoteAgentPreset,
} from "../types";
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

export type CreateRemoteAgentFormValues = {
  displayName: string;
  seatId: string;
  model: string;
  preset: RemoteAgentPreset;
  room: string;
  notes: string;
  arm: boolean;
};

type CreateRemoteAgentDialogProps = {
  open: boolean;
  defaultRoom: string;
  isPending: boolean;
  error: string | null;
  onOpenChange: (open: boolean) => void;
  onSubmit: (values: CreateRemoteAgentFormValues) => Promise<void>;
};

function slugifySeatId(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 63);
}

export function CreateRemoteAgentDialog({
  open,
  defaultRoom,
  isPending,
  error,
  onOpenChange,
  onSubmit,
}: CreateRemoteAgentDialogProps) {
  const [displayName, setDisplayName] = React.useState("");
  const [seatId, setSeatId] = React.useState("");
  const [seatTouched, setSeatTouched] = React.useState(false);
  const [model, setModel] = React.useState("gemma3:4b");
  const [customModel, setCustomModel] = React.useState("");
  const [preset, setPreset] = React.useState<RemoteAgentPreset>("co-lab-gemma");
  const [room, setRoom] = React.useState(defaultRoom);
  const [notes, setNotes] = React.useState("");
  const [arm, setArm] = React.useState(true);

  React.useEffect(() => {
    if (!open) return;
    setDisplayName("");
    setSeatId("");
    setSeatTouched(false);
    setModel("gemma3:4b");
    setCustomModel("");
    setPreset("co-lab-gemma");
    setRoom(defaultRoom);
    setNotes("");
    setArm(true);
  }, [open, defaultRoom]);

  React.useEffect(() => {
    if (!seatTouched && displayName) {
      setSeatId(slugifySeatId(displayName) || "remote-agent");
    }
  }, [displayName, seatTouched]);

  // When picking grok-4.5, default preset to watch-only until cortex lands
  React.useEffect(() => {
    if (model === "grok-4.5" && preset === "co-lab-gemma") {
      setPreset("co-lab-watch");
    }
  }, [model, preset]);

  const resolvedModel = model === "__custom__" ? customModel.trim() : model;
  const canSubmit =
    displayName.trim().length > 0 &&
    seatId.trim().length > 0 &&
    resolvedModel.length > 0 &&
    !isPending;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className="max-w-md"
        data-testid="create-remote-agent-dialog"
      >
        <DialogHeader>
          <DialogTitle>Create remote agent</DialogTitle>
          <DialogDescription>
            Register a host-pinned seat on the connected machine (same idea as
            local agents, but runs on home via host-agentd). Place stays honest
            — Arm/Stop and location proof apply after create.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-3 py-2">
          <div className="space-y-1.5">
            <label
              className="text-xs font-medium text-foreground"
              htmlFor="remote-agent-name"
            >
              Display name
            </label>
            <Input
              id="remote-agent-name"
              placeholder="Home Grok 4.5"
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
              autoFocus
            />
          </div>

          <div className="space-y-1.5">
            <label
              className="text-xs font-medium text-foreground"
              htmlFor="remote-agent-seat"
            >
              Seat id
            </label>
            <Input
              id="remote-agent-seat"
              className="font-mono text-xs"
              placeholder="home-grok-45"
              value={seatId}
              onChange={(e) => {
                setSeatTouched(true);
                setSeatId(e.target.value);
              }}
            />
            <p className="text-3xs text-muted-foreground">
              Stable id on the host (slug). Used for units and Remote Agents
              cards.
            </p>
          </div>

          <div className="space-y-1.5">
            <label
              className="text-xs font-medium text-foreground"
              htmlFor="remote-agent-model"
            >
              Model
            </label>
            <select
              id="remote-agent-model"
              className="h-9 w-full rounded-md border border-border/60 bg-background px-2 text-xs text-foreground"
              value={
                REMOTE_AGENT_MODEL_OPTIONS.some((m) => m.id === model)
                  ? model
                  : "__custom__"
              }
              onChange={(e) => setModel(e.target.value)}
            >
              {REMOTE_AGENT_MODEL_OPTIONS.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.label}
                </option>
              ))}
              <option value="__custom__">Custom model id…</option>
            </select>
            {model === "__custom__" ||
            !REMOTE_AGENT_MODEL_OPTIONS.some((m) => m.id === model) ? (
              <Input
                className="font-mono text-xs"
                placeholder="model-id"
                value={customModel || (model !== "__custom__" ? model : "")}
                onChange={(e) => {
                  setModel("__custom__");
                  setCustomModel(e.target.value);
                }}
              />
            ) : (
              <p className="text-3xs text-muted-foreground">
                {REMOTE_AGENT_MODEL_OPTIONS.find((m) => m.id === model)?.hint}
              </p>
            )}
          </div>

          <div className="space-y-1.5">
            <label
              className="text-xs font-medium text-foreground"
              htmlFor="remote-agent-preset"
            >
              Runtime preset
            </label>
            <select
              id="remote-agent-preset"
              className="h-9 w-full rounded-md border border-border/60 bg-background px-2 text-xs text-foreground"
              value={preset}
              onChange={(e) => setPreset(e.target.value as RemoteAgentPreset)}
            >
              {REMOTE_AGENT_PRESETS.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.label} — {p.description}
                </option>
              ))}
            </select>
          </div>

          <div className="space-y-1.5">
            <label
              className="text-xs font-medium text-foreground"
              htmlFor="remote-agent-room"
            >
              Room UUID
            </label>
            <Input
              id="remote-agent-room"
              className="font-mono text-xs"
              placeholder="agent-metabolism UUID"
              value={room}
              onChange={(e) => setRoom(e.target.value)}
            />
          </div>

          <div className="space-y-1.5">
            <label
              className="text-xs font-medium text-foreground"
              htmlFor="remote-agent-notes"
            >
              Notes (optional)
            </label>
            <Input
              id="remote-agent-notes"
              placeholder="e.g. Grok 4.5 internal seat on home"
              value={notes}
              onChange={(e) => setNotes(e.target.value)}
            />
          </div>

          <label className="flex items-center gap-2 text-xs text-foreground">
            <input
              type="checkbox"
              checked={arm}
              onChange={(e) => setArm(e.target.checked)}
              className="rounded border-border"
            />
            Arm after create (start unit on host)
          </label>

          {error ? (
            <p
              className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-2xs text-destructive"
              role="alert"
            >
              {error}
            </p>
          ) : null}
        </div>

        <DialogFooter className="gap-2 sm:gap-0">
          <Button
            type="button"
            variant="ghost"
            disabled={isPending}
            onClick={() => onOpenChange(false)}
          >
            Cancel
          </Button>
          <Button
            type="button"
            disabled={!canSubmit}
            onClick={() => {
              void onSubmit({
                displayName: displayName.trim(),
                seatId: seatId.trim(),
                model: resolvedModel,
                preset,
                room: room.trim(),
                notes: notes.trim(),
                arm,
              });
            }}
          >
            {isPending ? "Creating…" : "Create on host"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
