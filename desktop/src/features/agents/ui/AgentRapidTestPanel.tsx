import * as React from "react";

import { useChannelsQuery } from "@/features/channels/hooks";
import type {
  Channel,
  ManagedAgent,
  ManagedAgentRuntimeStatus,
} from "@/shared/api/types";

import { ManagedAgentLogPanel } from "./ManagedAgentLogPanel";
import {
  buildRapidTestPrompt,
  createSmokeId,
  filterEligibleRapidTestChannels,
  pickDefaultRapidTestChannelId,
  type RapidTestSelection,
} from "./agentRapidTest";

/**
 * Compact, low-glare surface for the in-app rapid agent smoke test.
 *
 * The panel never sends the smoke message itself — submission is owned by the
 * parent workflow (Save / restart / test), which composes the owner-authored
 * message (via `buildRapidTestPrompt`) and posts it through the same channel
 * send path as any other owner message. The panel only:
 *
 *   - Queries channels *only while open* (no idle network).
 *   - Filters down to channels that already contain the managed agent, so the
 *     smoke test always reaches the agent it claims to test.
 *   - Preserves the user's last valid default across refetches, but switches
 *     to a different eligible channel when the previous default becomes
 *     invalid (e.g. the agent was removed from the channel).
 *   - Exposes the selected channel id through `onChannelChange` so the parent
 *     can wire Save/restart/test to the right target.
 *   - Renders no-channel / loading / query-error states inline.
 *
 * The smoke id is minted lazily per opening and stays stable until the panel
 * closes so the rendered prompt and the posted message stay in sync.
 */

export type AgentRapidTestPanelProps = {
  agent: ManagedAgent | null;
  disabled?: boolean;
  /** Whether the panel is currently open. Channels are only queried when true. */
  open: boolean;
  /**
   * Notifies the parent whenever the selected eligible channel id changes.
   * Called with `null` when no eligible channel is available, so the parent
   * can disable Save/restart/test accordingly.
   */
  onSelectionChange?: (selection: RapidTestSelection | null) => void;
  /**
   * Optional override for the channel list (e.g. from parent-owned cached
   * data). When provided, the panel skips the channel query entirely while
   * still honouring `open` as the data source toggle.
   */
  channels?: readonly Channel[];
  /** Current pair status for the active community, when the agent is local. */
  runtime?: ManagedAgentRuntimeStatus | null;
  /** Bounded, backend-redacted log tail for the selected local agent. */
  logContent?: string | null;
  logError?: Error | null;
  logLoading?: boolean;
  /** Remote/provider-backed agents can still be edited, but not restarted here. */
  runtimeControlsAvailable?: boolean;
};

export function AgentRapidTestPanel({
  agent,
  disabled = false,
  open,
  onSelectionChange,
  channels: channelsOverride,
  runtime = null,
  logContent = null,
  logError = null,
  logLoading = false,
  runtimeControlsAvailable = false,
}: AgentRapidTestPanelProps) {
  const channelsQuery = useChannelsQuery({
    enabled: open && channelsOverride === undefined,
  });
  const channels = channelsOverride ?? channelsQuery.data ?? [];

  const eligibleChannels = React.useMemo(
    () => filterEligibleRapidTestChannels(channels, agent),
    [channels, agent],
  );

  const [selectedChannelId, setSelectedChannelId] = React.useState<
    string | null
  >(null);

  // The panel holds one smoke id so its rendered preview and posted prompt stay
  // identical throughout a single open run.
  const [smokeId, setSmokeId] = React.useState<string | null>(null);

  // Reopening or switching agents starts a fresh correlated smoke run. Reset
  // effects intentionally run before the seed effect below so an initial mount
  // cannot clear the id after it is created in the same passive-effect batch.
  React.useEffect(() => {
    if (!open) {
      setSelectedChannelId(null);
      setSmokeId(null);
    }
  }, [open]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: Agent identity is an intentional reset trigger.
  React.useEffect(() => {
    setSelectedChannelId(null);
    setSmokeId(null);
  }, [agent?.pubkey]);

  // Seed the smoke id once on first successful eligible selection so the
  // value the panel renders and the value the parent posts stay identical.
  React.useEffect(() => {
    if (smokeId) {
      return;
    }
    if (!open) {
      return;
    }
    if (eligibleChannels.length === 0) {
      return;
    }
    setSmokeId(createSmokeId());
  }, [smokeId, open, eligibleChannels.length]);

  // Resolve the current selection, preserving the user's previous valid pick
  // and falling back to the first eligible channel when needed.
  const resolvedChannelId = React.useMemo(
    () => pickDefaultRapidTestChannelId(eligibleChannels, selectedChannelId),
    [eligibleChannels, selectedChannelId],
  );

  // Keep local state aligned with the resolved id so the next memo recomputes
  // from a stable baseline (and so the user's pick survives a refetch).
  React.useEffect(() => {
    if (resolvedChannelId !== selectedChannelId) {
      setSelectedChannelId(resolvedChannelId);
    }
  }, [resolvedChannelId, selectedChannelId]);

  const selectedChannel = React.useMemo(
    () =>
      resolvedChannelId
        ? (eligibleChannels.find((c) => c.id === resolvedChannelId) ?? null)
        : null,
    [eligibleChannels, resolvedChannelId],
  );

  const promptPreview = React.useMemo(
    () => (smokeId ? buildRapidTestPrompt({ smokeId }) : null),
    [smokeId],
  );
  const selection = React.useMemo<RapidTestSelection | null>(
    () =>
      resolvedChannelId && promptPreview && selectedChannel
        ? {
            channel: selectedChannel,
            channelId: resolvedChannelId,
            prompt: promptPreview,
          }
        : null,
    [promptPreview, resolvedChannelId, selectedChannel],
  );

  React.useEffect(() => {
    if (open) {
      onSelectionChange?.(selection);
    }
  }, [onSelectionChange, open, selection]);

  const handleSelectChannel = React.useCallback(
    (event: React.ChangeEvent<HTMLSelectElement>) => {
      setSelectedChannelId(event.target.value || null);
    },
    [],
  );

  // ---- Render helpers --------------------------------------------------

  let body: React.ReactNode;
  if (!open) {
    return null;
  }

  const runtimeLabel = runtime?.lifecycle
    ? runtime.lifecycle[0].toUpperCase() + runtime.lifecycle.slice(1)
    : "Not started";
  const runtimeFailure =
    runtime?.lifecycle === "failed"
      ? "Runtime reported a failure. Inspect the bounded log tail below."
      : runtime?.lifecycle === "stopped"
        ? "Runtime is stopped. Save & restart will start only this agent pair."
        : null;

  if (channelsQuery.isLoading && !channelsOverride) {
    body = <p className="text-xs text-muted-foreground">Loading channels…</p>;
  } else if (channelsQuery.error instanceof Error && !channelsOverride) {
    body = (
      <p className="rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
        Could not load eligible channels for the active community.
      </p>
    );
  } else if (eligibleChannels.length === 0) {
    body = (
      <p className="text-xs text-muted-foreground">
        No eligible channels. Add this agent to a channel (DMs are allowed) and
        try again.
      </p>
    );
  } else {
    body = (
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium"
          htmlFor="agent-rapid-test-channel"
        >
          Channel
        </label>
        <select
          className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
          data-testid="agent-rapid-test-channel"
          disabled={disabled}
          id="agent-rapid-test-channel"
          onChange={handleSelectChannel}
          value={resolvedChannelId ?? ""}
        >
          {eligibleChannels.map((channel) => (
            <option key={channel.id} value={channel.id}>
              {channel.name} · {channel.channelType}
            </option>
          ))}
        </select>
        <p className="text-xs text-muted-foreground">
          Only channels where you are a member and the agent is already a member
          are listed. DMs are allowed.
        </p>
      </div>
    );
  }

  return (
    <section
      aria-label="Rapid agent smoke test"
      className="space-y-3 rounded-2xl border border-border/70 bg-muted/20 p-4"
    >
      <header className="space-y-1">
        <p className="text-sm font-semibold tracking-tight">
          Rapid agent smoke test
        </p>
        <p className="text-xs text-muted-foreground">
          Save updates the selected agent. Save & restart applies the change to
          this local pair. Save, restart & test additionally posts a visible
          message authored as the owner in the selected channel and opens its
          thread. The{" "}
          <code className="rounded bg-background/80 px-1 py-0.5 text-[11px]">
            BUZZ_HERMES_OK
          </code>{" "}
          directive is the only token the agent must echo back; no env values or
          secrets are embedded. Other members of that channel will see the
          labelled test message.
        </p>
      </header>

      <section
        aria-label="Managed agent runtime status"
        className="space-y-1 rounded-xl border border-border/70 bg-background/60 px-3 py-2"
      >
        <div className="flex items-center justify-between gap-3 text-xs">
          <span className="font-medium">Runtime status</span>
          <span
            className="rounded-full bg-muted px-2 py-0.5 font-medium"
            data-testid="agent-rapid-runtime-status"
          >
            {runtimeLabel}
          </span>
        </div>
        {!runtimeControlsAvailable ? (
          <p className="text-xs text-muted-foreground">
            Restart and smoke testing are available only for local agents in the
            active community.
          </p>
        ) : runtimeFailure ? (
          <p className="text-xs text-destructive">{runtimeFailure}</p>
        ) : (
          <p className="text-xs text-muted-foreground">
            Save changes, restart this pair, or post a labelled owner-authored
            smoke prompt from the footer.
          </p>
        )}
      </section>

      {body}

      {selectedChannel && promptPreview ? (
        <div className="space-y-1.5">
          <p className="text-xs font-medium text-muted-foreground">
            Smoke prompt preview
          </p>
          <pre className="max-h-40 overflow-auto rounded-xl border border-border/70 bg-background/80 px-3 py-2 text-[11px] leading-snug text-foreground">
            {promptPreview.body}
          </pre>
          <p className="text-[11px] text-muted-foreground">
            Smoke id:{" "}
            <code className="rounded bg-background/80 px-1 py-0.5">
              {promptPreview.smokeId}
            </code>
          </p>
        </div>
      ) : null}

      {runtimeControlsAvailable ? (
        <details className="rounded-xl border border-border/60 bg-background/70 p-3">
          <summary className="cursor-pointer text-xs font-medium text-foreground">
            Recent harness log
          </summary>
          <div className="mt-3 max-h-64 overflow-auto">
            <ManagedAgentLogPanel
              chrome="bare"
              error={logError}
              isLoading={logLoading}
              logContent={logContent}
              selectedAgent={agent}
              variant="inline"
            />
          </div>
        </details>
      ) : null}
    </section>
  );
}
