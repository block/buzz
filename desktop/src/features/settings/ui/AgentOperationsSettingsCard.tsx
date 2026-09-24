import { useEffect, useMemo, useState } from "react";
import { toast } from "sonner";

import { getChannels } from "@/shared/api/tauriChannels";
import { listManagedAgents } from "@/shared/api/tauri";
import {
  getAgentOperationsStatus,
  saveAgentOperationsConfig,
} from "@/shared/api/tauriAgentOperations";
import type {
  AgentOperationsConfig,
  Channel,
  ManagedAgent,
} from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { Switch } from "@/shared/ui/switch";

import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

const SCHEDULE =
  "Daily at 09:00 Asia/Manila; liveness every 30 seconds while Buzz Desktop is running";

const EMPTY_CONFIG: AgentOperationsConfig = {
  enabled: false,
  channelId: null,
  assistantPubkey: null,
};

export function AgentOperationsSettingsCard() {
  const [config, setConfig] = useState<AgentOperationsConfig>(EMPTY_CONFIG);
  const [channels, setChannels] = useState<Channel[]>([]);
  const [agents, setAgents] = useState<ManagedAgent[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<string>("");

  useEffect(() => {
    let active = true;
    void Promise.all([
      getAgentOperationsStatus(),
      getChannels(null),
      listManagedAgents(),
    ])
      .then(([status, channelPayload, managedAgents]) => {
        if (!active) return;
        setConfig(status.config);
        setChannels(
          (channelPayload.channels ?? []).filter((channel) => channel.isMember),
        );
        setAgents(
          managedAgents.filter(
            (agent) =>
              agent.backend.type === "local" && agent.pubkey.trim().length > 0,
          ),
        );
        setMessage("");
      })
      .catch((error: unknown) => {
        if (!active) return;
        setMessage(
          error instanceof Error
            ? error.message
            : "Operations automation settings could not be loaded.",
        );
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, []);

  const selectedChannelIsStale = useMemo(
    () =>
      Boolean(config.channelId) &&
      !channels.some((channel) => channel.id === config.channelId),
    [channels, config.channelId],
  );
  const selectedAgentIsStale = useMemo(
    () =>
      Boolean(config.assistantPubkey) &&
      !agents.some((agent) => agent.pubkey === config.assistantPubkey),
    [agents, config.assistantPubkey],
  );

  async function save() {
    if (config.enabled && (!config.channelId || !config.assistantPubkey)) {
      setMessage(
        "Choose both a destination channel and a local assistant before enabling.",
      );
      return;
    }
    setSaving(true);
    setMessage("Saving operations automation settings…");
    try {
      const status = await saveAgentOperationsConfig(config);
      setConfig(status.config);
      setMessage(
        status.config.enabled
          ? "Operations automation is enabled. The first scan may alert for an agent that is already unhealthy."
          : "Operations automation is disabled. No new wake or alert attempts will be made.",
      );
      toast.success(
        status.config.enabled
          ? "Operations automation enabled"
          : "Operations automation disabled",
      );
    } catch (error) {
      setMessage(
        error instanceof Error
          ? error.message
          : "Operations automation could not be saved.",
      );
    } finally {
      setSaving(false);
    }
  }

  return (
    <SettingsOptionGroup
      data-testid="settings-agent-operations"
      description="Posts a daily operations wake and alerts once per Start-on-launch outage episode. Configuration persists locally on this device."
      title="Operations automation"
    >
      <SettingsOptionRow>
        <div className="min-w-0">
          <label
            className="font-medium text-foreground"
            htmlFor="agent-operations-enabled"
          >
            Enable
          </label>
          <p
            className="mt-0.5 text-sm text-muted-foreground/70"
            id="agent-operations-enable-help"
          >
            Uses the exact selected channel and assistant identities. An
            already-unhealthy agent may alert on the first scan.
          </p>
        </div>
        <Switch
          aria-describedby="agent-operations-enable-help"
          checked={config.enabled}
          disabled={loading || saving}
          id="agent-operations-enabled"
          onCheckedChange={(enabled) =>
            setConfig((current) => ({ ...current, enabled }))
          }
        />
      </SettingsOptionRow>

      <SettingsOptionRow className="items-start">
        <div className="min-w-0 flex-1">
          <label
            className="font-medium text-foreground"
            htmlFor="agent-operations-channel"
          >
            Destination channel
          </label>
          <p
            className="mt-0.5 text-sm text-muted-foreground/70"
            id="agent-operations-channel-help"
          >
            Joined channels only. Membership is checked again when you save.
          </p>
        </div>
        <select
          aria-describedby="agent-operations-channel-help"
          className="min-h-9 w-full max-w-72 rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          disabled={loading || saving}
          id="agent-operations-channel"
          onChange={(event) =>
            setConfig((current) => ({
              ...current,
              channelId: event.target.value || null,
            }))
          }
          value={config.channelId ?? ""}
        >
          <option value="">Select a joined channel</option>
          {selectedChannelIsStale ? (
            <option value={config.channelId ?? ""}>
              Previously selected channel (unavailable)
            </option>
          ) : null}
          {channels.map((channel) => (
            <option key={channel.id} value={channel.id}>
              {channel.name}
            </option>
          ))}
        </select>
      </SettingsOptionRow>

      <SettingsOptionRow className="items-start">
        <div className="min-w-0 flex-1">
          <label
            className="font-medium text-foreground"
            htmlFor="agent-operations-assistant"
          >
            Digest and alert assistant
          </label>
          <p
            className="mt-0.5 text-sm text-muted-foreground/70"
            id="agent-operations-assistant-help"
          >
            Instantiated local managed agents with an exact signing identity.
          </p>
        </div>
        <select
          aria-describedby="agent-operations-assistant-help"
          className="min-h-9 w-full max-w-72 rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          disabled={loading || saving}
          id="agent-operations-assistant"
          onChange={(event) =>
            setConfig((current) => ({
              ...current,
              assistantPubkey: event.target.value || null,
            }))
          }
          value={config.assistantPubkey ?? ""}
        >
          <option value="">Select a local assistant</option>
          {selectedAgentIsStale ? (
            <option value={config.assistantPubkey ?? ""}>
              Previously selected assistant (unavailable)
            </option>
          ) : null}
          {agents.map((agent) => (
            <option key={agent.pubkey} value={agent.pubkey}>
              {agent.name}
            </option>
          ))}
        </select>
      </SettingsOptionRow>

      <SettingsOptionRow>
        <div className="min-w-0">
          <p className="font-medium text-foreground">Schedule</p>
          <p className="mt-0.5 text-sm text-muted-foreground/70">{SCHEDULE}</p>
        </div>
      </SettingsOptionRow>

      <div className="flex flex-col gap-3 px-4 py-4 sm:flex-row sm:items-center sm:justify-between">
        <p
          aria-live="polite"
          className="min-w-0 text-sm text-muted-foreground"
          role="status"
        >
          {loading ? "Loading operations automation settings…" : message}
        </p>
        <Button
          disabled={loading || saving}
          onClick={() => void save()}
          type="button"
        >
          {saving ? "Saving…" : "Save settings"}
        </Button>
      </div>
    </SettingsOptionGroup>
  );
}
