import * as React from "react";
import { RefreshCw, Settings2 } from "lucide-react";

import { useRemoteHostAgents } from "../useRemoteHostAgents";
import { REMOTE_AGENT_PRESETS, type RemoteAgentPreset } from "../types";
import { RemoteAgentCard } from "./RemoteAgentCard";
import { RemoteHostSettingsDialog } from "./RemoteHostSettingsDialog";
import { CreateRemoteAgentDialog } from "./CreateRemoteAgentDialog";
import { CreateIdentityCard } from "@/features/agents/ui/CreateIdentityCard";
import { IDENTITY_CARD_GRID_CLASS } from "@/features/agents/ui/UnifiedAgentsSection";
import { Button } from "@/shared/ui/button";
import { SectionHeader } from "@/shared/ui/PageHeader";

const FALLBACK_ROOM = "92297894-c2e8-4df1-a710-d1cfd1032d5e";

export function RemoteAgentsSection() {
  const remote = useRemoteHostAgents();
  const [settingsOpen, setSettingsOpen] = React.useState(false);
  const [createOpen, setCreateOpen] = React.useState(false);
  const [createError, setCreateError] = React.useState<string | null>(null);
  const [preset, setPreset] = React.useState<RemoteAgentPreset>("co-lab-gemma");
  const armRoom = remote.connection?.defaultRoom?.trim() || FALLBACK_ROOM;

  return (
    <section className="relative space-y-4" data-testid="remote-agents-section">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <SectionHeader
          title="Remote Agents"
          description="Host-pinned seats on your always-on machine (headless home). Not local Desktop agents — place + arm/disarm via host-agentd."
        />
        <div className="flex shrink-0 flex-wrap items-center gap-2">
          <select
            aria-label="Arm preset"
            className="h-9 max-w-[12rem] rounded-md border border-border/60 bg-background px-2 text-xs text-foreground"
            value={preset}
            onChange={(e) => setPreset(e.target.value as RemoteAgentPreset)}
          >
            {REMOTE_AGENT_PRESETS.map((p) => (
              <option key={p.id} value={p.id}>
                {p.label}
              </option>
            ))}
          </select>
          <Button
            className="h-9 gap-1.5"
            size="sm"
            type="button"
            variant="outline"
            disabled={remote.isLoading || !remote.connection}
            onClick={() => void remote.refresh()}
          >
            <RefreshCw
              className={`h-3.5 w-3.5 ${remote.isLoading ? "animate-spin" : ""}`}
            />
            Refresh
          </Button>
          <Button
            className="h-9 gap-1.5"
            size="sm"
            type="button"
            variant="secondary"
            onClick={() => setSettingsOpen(true)}
          >
            <Settings2 className="h-3.5 w-3.5" />
            Host
          </Button>
        </div>
      </div>

      {remote.connection ? (
        <p className="text-2xs text-muted-foreground">
          {remote.status ? "Connected to" : "Configured host"}{" "}
          <span className="font-medium text-foreground/90">
            {remote.connection.label}
          </span>{" "}
          · <span className="font-mono">{remote.connection.baseUrl}</span>
          {remote.status?.host_id ? ` · host ${remote.status.host_id}` : null}
          {remote.status?.ollama?.ok
            ? ` · ollama ${(remote.status.ollama.models || []).join(",") || "ok"}`
            : null}
          {remote.locationProof?.schema
            ? ` · proof ${String(remote.locationProof.schema)}`
            : null}
          {!remote.status && !remote.isLoading
            ? " · waiting for host-agentd (check Tailscale + base URL)"
            : null}
        </p>
      ) : (
        <p className="text-2xs text-muted-foreground">
          No host configured. Click <strong>Host</strong> and set the home
          Tailscale base URL (e.g.{" "}
          <code className="text-3xs">http://100.79.175.63:8787</code>) + token
          from DM. Shell access:{" "}
          <code className="text-3xs">ssh asus@asus-g501vw</code>.
        </p>
      )}

      {remote.error ? (
        <p
          className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-2xs text-destructive"
          role="alert"
        >
          {remote.error}
        </p>
      ) : null}
      {remote.notice ? (
        <p className="rounded-md border border-border/50 bg-muted/30 px-3 py-2 text-2xs text-muted-foreground">
          {remote.notice}
        </p>
      ) : null}

      <div className={IDENTITY_CARD_GRID_CLASS}>
        {remote.cards.map((card) => (
          <RemoteAgentCard
            key={`${card.hostId}-${card.seatId}`}
            card={card}
            defaultPreset={preset}
            isPending={remote.isPending && remote.pendingSeat === card.seatId}
            onArm={() => {
              void remote.arm(card.seatId, preset, armRoom);
            }}
            onDisarm={() => {
              void remote.disarm(card.seatId, preset);
            }}
          />
        ))}
        {remote.connection ? (
          <CreateIdentityCard
            ariaLabel="Create remote agent on host"
            dataTestId="create-remote-agent-card"
            label="New remote"
            onClick={() => {
              setCreateError(null);
              setCreateOpen(true);
            }}
          />
        ) : (
          <button
            className="flex min-h-[140px] flex-col items-center justify-center rounded-xl border border-dashed border-border/70 bg-muted/10 p-4 text-center text-sm text-muted-foreground transition-colors hover:bg-muted/25"
            type="button"
            onClick={() => setSettingsOpen(true)}
          >
            + Connect host
          </button>
        )}
      </div>

      <RemoteHostSettingsDialog
        open={settingsOpen}
        initial={remote.connection}
        onOpenChange={setSettingsOpen}
        onSave={remote.saveConnection}
        onClear={remote.clearConnection}
      />

      <CreateRemoteAgentDialog
        open={createOpen}
        defaultRoom={armRoom}
        isPending={remote.isPending}
        error={createError}
        onOpenChange={(open) => {
          setCreateOpen(open);
          if (!open) setCreateError(null);
        }}
        onSubmit={async (values) => {
          setCreateError(null);
          try {
            await remote.createAgent({
              displayName: values.displayName,
              seatId: values.seatId,
              model: values.model,
              preset: values.preset,
              room: values.room,
              notes: values.notes,
              arm: values.arm,
            });
            setCreateOpen(false);
          } catch (err) {
            setCreateError(
              err instanceof Error ? err.message : "Create failed",
            );
          }
        }}
      />
    </section>
  );
}
