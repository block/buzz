import { Archive, Trash2 } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useTranslation } from "@/i18n";
import {
  createSaveSubscription,
  deleteSaveSubscription,
  listSaveSubscriptions,
  mergeSaveSubscriptionKinds,
  removeSaveSubscriptionKind,
  type SaveSubscription,
  type ScopeType,
} from "@/shared/api/tauriArchive";
import {
  KIND_AGENT_OBSERVER_FRAME,
  KIND_AGENT_TURN_METRIC,
} from "@/shared/constants/kinds";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import { Switch } from "@/shared/ui/switch";
import {
  SettingsOptionGroup,
  SettingsOptionRow,
} from "@/features/settings/ui/SettingsOptionGroup";
import { SettingsSectionHeader } from "@/features/settings/ui/SettingsSectionHeader";
import { setExplicitAgentMetricArchiveChoice } from "../agentMetricArchivePreference";
import { setExplicitObserverArchiveChoice } from "../observerArchivePreference";

import {
  buildSubscriptionRequest,
  isGroupFullyChecked,
  isGroupIndeterminate,
  KIND_GROUPS,
  parseCustomKinds,
  toggleGroup,
  toggleKind,
} from "./localArchiveKinds";

// ── Helpers ───────────────────────────────────────────────────────────────────

function scopeLabel(
  sub: SaveSubscription,
  channelNameById: Map<string, string>,
): string {
  if (sub.scopeType === "channel_h") {
    return channelNameById.get(sub.scopeValue) ?? sub.scopeValue;
  }
  if (sub.scopeType === "owner_p") {
    if (sub.kinds.includes(KIND_AGENT_TURN_METRIC)) {
      return "My agents' turn metrics";
    }
    return "My agent session frames";
  }
  return sub.scopeValue;
}

function kindSummary(kinds: number[]): string {
  if (kinds.length === 0) return "no kinds";
  if (kinds.length <= 4) return kinds.join(", ");
  return `${kinds.slice(0, 3).join(", ")} +${kinds.length - 3} more`;
}

// ── Observer-feed archive section ─────────────────────────────────────────────

type ObserverSectionProps = {
  enabled: boolean;
  toggling: boolean;
  onToggle: (checked: boolean) => void;
};

function ObserverArchiveSection({
  enabled,
  toggling,
  onToggle,
}: ObserverSectionProps) {
  const { t } = useTranslation();
  const toggleDisabled = toggling;
  return (
    <div data-testid="local-archive-observer-section">
      <SettingsOptionGroup
        title={t("local-archive.settings-card.observer-feed")}
      >
        <SettingsOptionRow>
          <div className="min-w-0 flex-1">
            <label
              className="text-sm font-medium"
              htmlFor="local-archive-observer-toggle"
            >
              {t("local-archive.settings-card.observer-toggle")}
            </label>
            <p
              className="text-sm font-normal text-muted-foreground/70"
              data-settings-subcopy
            >
              {t("local-archive.settings-card.observer-description", {
                kind: KIND_AGENT_OBSERVER_FRAME,
              })}
            </p>
          </div>
          <Switch
            checked={enabled}
            data-testid="local-archive-observer-toggle"
            disabled={toggleDisabled}
            id="local-archive-observer-toggle"
            onCheckedChange={onToggle}
          />
        </SettingsOptionRow>
      </SettingsOptionGroup>
    </div>
  );
}

// ── Agent-turn-metric archive section ────────────────────────────────────────

type AgentMetricSectionProps = {
  enabled: boolean;
  toggling: boolean;
  onToggle: (checked: boolean) => void;
};

function AgentMetricArchiveSection({
  enabled,
  toggling,
  onToggle,
}: AgentMetricSectionProps) {
  const { t } = useTranslation();
  return (
    <div data-testid="local-archive-agent-metric-section">
      <SettingsOptionGroup
        title={t("local-archive.settings-card.turn-metrics")}
      >
        <SettingsOptionRow>
          <div className="min-w-0 flex-1">
            <label
              className="text-sm font-medium"
              htmlFor="local-archive-agent-metric-toggle"
            >
              {t("local-archive.settings-card.turn-metrics-toggle")}
            </label>
            <p
              className="text-sm font-normal text-muted-foreground/70"
              data-settings-subcopy
            >
              {t("local-archive.settings-card.turn-metrics-description", {
                kind: KIND_AGENT_TURN_METRIC,
              })}
            </p>
          </div>
          <Switch
            checked={enabled}
            data-testid="local-archive-agent-metric-toggle"
            disabled={toggling}
            id="local-archive-agent-metric-toggle"
            onCheckedChange={onToggle}
          />
        </SettingsOptionRow>
      </SettingsOptionGroup>
    </div>
  );
}

// ── Add-subscription form ─────────────────────────────────────────────────────

type KindChecklistProps = {
  checkedKinds: ReadonlySet<number>;
  onChange: (next: Set<number>) => void;
};

function KindChecklist({ checkedKinds, onChange }: KindChecklistProps) {
  const { t } = useTranslation();
  return (
    <div className="space-y-4">
      {KIND_GROUPS.map((group) => {
        const fullyChecked = isGroupFullyChecked(group, checkedKinds);
        const indeterminate = isGroupIndeterminate(group, checkedKinds);
        return (
          <div key={group.label}>
            {/* Group header */}
            <div className="mb-1.5 flex items-center gap-2">
              <Checkbox
                checked={indeterminate ? "indeterminate" : fullyChecked}
                data-testid={`local-archive-group-${group.label}`}
                id={`local-archive-group-${group.label}`}
                onCheckedChange={() =>
                  onChange(toggleGroup(group, checkedKinds))
                }
              />
              <label
                className="cursor-pointer text-sm font-medium"
                htmlFor={`local-archive-group-${group.label}`}
              >
                {t(group.labelKey)}
              </label>
            </div>
            {/* Individual kind checkboxes */}
            <div className="ml-6 space-y-1.5">
              {group.items.map(({ kind, labelKey }) => (
                <div key={kind} className="flex items-center gap-2">
                  <Checkbox
                    checked={checkedKinds.has(kind)}
                    data-testid={`local-archive-kind-${kind}`}
                    id={`local-archive-kind-${kind}`}
                    onCheckedChange={() =>
                      onChange(toggleKind(kind, checkedKinds))
                    }
                  />
                  <label
                    className="cursor-pointer text-sm text-muted-foreground"
                    htmlFor={`local-archive-kind-${kind}`}
                  >
                    {t(labelKey, { kind })}
                  </label>
                </div>
              ))}
            </div>
          </div>
        );
      })}
    </div>
  );
}

// ── Custom kinds input ────────────────────────────────────────────────────────

type CustomKindsInputProps = {
  value: string;
  onChange: (raw: string) => void;
};

function CustomKindsInput({ value, onChange }: CustomKindsInputProps) {
  const { t } = useTranslation();
  const { invalid } = parseCustomKinds(value);
  const hasInvalid = invalid.length > 0;
  return (
    <div>
      <label
        className="mb-1.5 block text-sm font-medium"
        htmlFor="local-archive-custom-kinds"
      >
        {t("local-archive.settings-card.custom-kinds")}
      </label>
      <input
        className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        data-testid="local-archive-custom-kinds"
        id="local-archive-custom-kinds"
        onChange={(e) => onChange(e.target.value)}
        placeholder="e.g. 30023 1337"
        type="text"
        value={value}
      />
      <p
        className="mt-1 text-xs text-muted-foreground/70"
        data-settings-subcopy
      >
        {t("local-archive.settings-card.custom-kinds-help")}
      </p>
      {hasInvalid && (
        <p
          className="mt-1 text-xs text-destructive"
          data-testid="local-archive-custom-kinds-error"
        >
          {t("local-archive.settings-card.invalid-tokens")}{" "}
          {invalid.map((token, i) => (
            <React.Fragment key={token}>
              {i > 0 && ", "}
              <code className="font-mono">{token}</code>
            </React.Fragment>
          ))}
        </p>
      )}
    </div>
  );
}

// ── Add-subscription form (Steps 1 + 2) ──────────────────────────────────────

type AddFormProps = {
  channels: Array<{ id: string; name: string }>;
  title?: React.ReactNode;
  onSaved: () => void;
  onCancel: () => void;
};

function AddSubscriptionForm({
  channels,
  title,
  onSaved,
  onCancel,
}: AddFormProps) {
  const { t } = useTranslation();
  const [selectedChannelId, setSelectedChannelId] = React.useState("");
  const [checkedKinds, setCheckedKinds] = React.useState<Set<number>>(
    new Set(),
  );
  const [customKindsRaw, setCustomKindsRaw] = React.useState("");
  const [isAdding, setIsAdding] = React.useState(false);

  const { valid: customKinds } = parseCustomKinds(customKindsRaw);
  const request = buildSubscriptionRequest(
    "channel_h",
    selectedChannelId,
    checkedKinds,
    customKinds,
  );
  const canAdd = request !== null;

  const handleAdd = React.useCallback(async () => {
    if (request === null) return;

    setIsAdding(true);
    try {
      await createSaveSubscription(
        request.scopeType,
        request.scopeValue,
        request.kinds,
      );
      onSaved();
      toast.success(t("local-archive.settings-card.subscription-created"));
    } catch (err) {
      toast.error(
        err instanceof Error
          ? err.message
          : t("local-archive.settings-card.create-failed"),
      );
    } finally {
      setIsAdding(false);
    }
  }, [request, onSaved, t]);

  const handleCancel = () => {
    setSelectedChannelId("");
    setCheckedKinds(new Set());
    setCustomKindsRaw("");
    onCancel();
  };

  return (
    <SettingsOptionGroup title={title}>
      <div className="space-y-5 px-4 py-4">
        {/* Channel picker */}
        <div>
          <label
            className="mb-1.5 block text-sm font-medium"
            htmlFor="local-archive-channel-select"
          >
            {t("local-archive.settings-card.channel")}
          </label>
          <select
            className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            data-testid="local-archive-channel-select"
            id="local-archive-channel-select"
            onChange={(e) => setSelectedChannelId(e.target.value)}
            value={selectedChannelId}
          >
            <option value="">
              {t("local-archive.settings-card.select-channel")}
            </option>
            {channels.map((ch) => (
              <option key={ch.id} value={ch.id}>
                {ch.name}
              </option>
            ))}
          </select>
        </div>

        {/* Event types (per-kind checklist) */}
        <div>
          <p className="mb-3 text-sm font-medium">
            {t("local-archive.settings-card.event-types")}
          </p>
          <KindChecklist
            checkedKinds={checkedKinds}
            onChange={setCheckedKinds}
          />
        </div>

        {/* Advanced: custom kinds */}
        <CustomKindsInput onChange={setCustomKindsRaw} value={customKindsRaw} />

        <div className="flex justify-end gap-2">
          <Button
            disabled={isAdding}
            onClick={handleCancel}
            type="button"
            variant="outline"
          >
            {t("sidebar.common.cancel")}
          </Button>
          <Button
            data-testid="local-archive-confirm-add"
            disabled={isAdding || !canAdd}
            onClick={() => void handleAdd()}
            type="button"
          >
            {isAdding
              ? t("onboarding.setup.saving")
              : t("sidebar.sections.confirm-save")}
          </Button>
        </div>
      </div>
    </SettingsOptionGroup>
  );
}

// ── Main component ────────────────────────────────────────────────────────────

export function LocalArchiveSettingsCard() {
  const { t } = useTranslation();
  const identityQuery = useIdentityQuery();
  const channelsQuery = useChannelsQuery();
  const [subs, setSubs] = React.useState<SaveSubscription[]>([]);
  const [isLoading, setIsLoading] = React.useState(true);
  const [deletingKey, setDeletingKey] = React.useState<string | null>(null);
  const [isAddingOpen, setIsAddingOpen] = React.useState(false);
  const [observerToggling, setObserverToggling] = React.useState(false);
  const [metricToggling, setMetricToggling] = React.useState(false);

  const pubkey = identityQuery.data?.pubkey ?? "";

  const channelNameById = React.useMemo<Map<string, string>>(() => {
    const map = new Map<string, string>();
    for (const ch of channelsQuery.data ?? []) {
      map.set(ch.id, ch.name);
    }
    return map;
  }, [channelsQuery.data]);

  const joinedChannels = React.useMemo(
    () => (channelsQuery.data ?? []).filter((ch) => ch.isMember),
    [channelsQuery.data],
  );

  const reload = React.useCallback(async () => {
    try {
      const rows = await listSaveSubscriptions();
      setSubs(rows);
    } catch (err) {
      console.warn("[LocalArchiveSettingsCard] list failed:", err);
    } finally {
      setIsLoading(false);
    }
  }, []);

  React.useEffect(() => {
    void reload();
  }, [reload]);

  const handleDelete = React.useCallback(
    async (scopeType: ScopeType, scopeValue: string) => {
      const key = `${scopeType}:${scopeValue}`;
      setDeletingKey(key);
      try {
        await deleteSaveSubscription(scopeType, scopeValue);
        await reload();
        toast.success(t("local-archive.settings-card.subscription-removed"));
      } catch (err) {
        toast.error(
          err instanceof Error
            ? err.message
            : t("local-archive.settings-card.remove-failed"),
        );
      } finally {
        setDeletingKey(null);
      }
    },
    [reload, t],
  );

  const observerEnabled = subs.some(
    (s) =>
      s.scopeType === "owner_p" && s.kinds.includes(KIND_AGENT_OBSERVER_FRAME),
  );
  const metricEnabled = subs.some(
    (s) =>
      s.scopeType === "owner_p" && s.kinds.includes(KIND_AGENT_TURN_METRIC),
  );

  const handleObserverToggle = React.useCallback(
    async (checked: boolean) => {
      if (!pubkey) return;
      setObserverToggling(true);
      try {
        if (checked) {
          await mergeSaveSubscriptionKinds(KIND_AGENT_OBSERVER_FRAME);
        } else {
          await removeSaveSubscriptionKind(KIND_AGENT_OBSERVER_FRAME);
        }
        setExplicitObserverArchiveChoice(pubkey, checked);
        toast.success(
          checked
            ? t("local-archive.settings-card.observer-enabled")
            : t("local-archive.settings-card.observer-disabled"),
        );
        await reload();
      } catch (err) {
        toast.error(
          err instanceof Error
            ? err.message
            : t("local-archive.settings-card.observer-update-failed"),
        );
      } finally {
        setObserverToggling(false);
      }
    },
    [pubkey, reload, t],
  );

  const handleMetricToggle = React.useCallback(
    async (checked: boolean) => {
      if (!pubkey) return;
      setMetricToggling(true);
      try {
        if (checked) {
          await mergeSaveSubscriptionKinds(KIND_AGENT_TURN_METRIC);
        } else {
          await removeSaveSubscriptionKind(KIND_AGENT_TURN_METRIC);
        }
        setExplicitAgentMetricArchiveChoice(pubkey, checked);
        toast.success(
          checked
            ? t("local-archive.settings-card.turn-metrics-enabled")
            : t("local-archive.settings-card.turn-metrics-disabled"),
        );
        await reload();
      } catch (err) {
        toast.error(
          err instanceof Error
            ? err.message
            : t("local-archive.settings-card.turn-metrics-update-failed"),
        );
      } finally {
        setMetricToggling(false);
      }
    },
    [pubkey, reload, t],
  );

  // Non-owner_p subscriptions shown in the active-subscriptions list.
  // observer (24200) and metric (44200) owner_p subs each have their own
  // dedicated section above.
  const channelSubs = subs.filter((s) => s.scopeType !== "owner_p");

  return (
    <section className="min-w-0" data-testid="settings-local-archive">
      <SettingsSectionHeader
        title={t("local-archive.settings-card.title")}
        description={t("local-archive.settings-card.description")}
      />

      <div className="space-y-6">
        {/* Observer-feed archive — dedicated first-class section */}
        <ObserverArchiveSection
          enabled={observerEnabled}
          onToggle={(checked) => void handleObserverToggle(checked)}
          toggling={observerToggling}
        />

        {/* Agent-turn-metric archive — dedicated first-class section */}
        <AgentMetricArchiveSection
          enabled={metricEnabled}
          onToggle={(checked) => void handleMetricToggle(checked)}
          toggling={metricToggling}
        />

        {/* Channel subscriptions */}
        <div data-testid="local-archive-subscriptions">
          {isLoading ? (
            <SettingsOptionGroup
              title={t("local-archive.settings-card.channel-subs")}
            >
              <div className="px-4 py-3 text-sm font-normal text-muted-foreground">
                {t("settings.signout.loading")}
              </div>
            </SettingsOptionGroup>
          ) : channelSubs.length === 0 ? (
            <SettingsOptionGroup
              title={t("local-archive.settings-card.channel-subs")}
            >
              <div className="px-4 py-3 text-sm font-normal text-muted-foreground">
                {t("local-archive.settings-card.no-subs")}
              </div>
            </SettingsOptionGroup>
          ) : (
            <SettingsOptionGroup
              title={t("local-archive.settings-card.channel-subs-count", {
                count: channelSubs.length,
              })}
            >
              {channelSubs.map((sub) => {
                const key = `${sub.scopeType}:${sub.scopeValue}`;
                return (
                  <div
                    key={key}
                    className="flex items-center gap-3 px-4 py-3"
                    data-testid={`local-archive-sub-${key}`}
                  >
                    <Archive className="h-4 w-4 shrink-0 text-muted-foreground" />
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-sm font-medium">
                        {scopeLabel(sub, channelNameById)}
                      </p>
                      <p
                        className="text-xs text-muted-foreground/70"
                        data-settings-subcopy
                      >
                        {sub.scopeType} · kinds: {kindSummary(sub.kinds)}
                      </p>
                    </div>
                    <Button
                      aria-label={t("local-archive.settings-card.remove-aria", {
                        name: scopeLabel(sub, channelNameById),
                      })}
                      disabled={deletingKey === key}
                      onClick={() =>
                        void handleDelete(sub.scopeType, sub.scopeValue)
                      }
                      size="icon"
                      variant="ghost"
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </div>
                );
              })}
            </SettingsOptionGroup>
          )}
        </div>

        {/* Add channel subscription */}
        <div data-testid="local-archive-add">
          {isAddingOpen ? (
            <AddSubscriptionForm
              channels={joinedChannels}
              onCancel={() => setIsAddingOpen(false)}
              onSaved={() => {
                setIsAddingOpen(false);
                void reload();
              }}
              title={t("local-archive.settings-card.add-subscription")}
            />
          ) : (
            <SettingsOptionGroup
              title={t("local-archive.settings-card.add-subscription")}
            >
              <SettingsOptionRow>
                <div className="min-w-0 flex-1">
                  <p className="text-sm font-medium">
                    {t("local-archive.settings-card.subscribe")}
                  </p>
                  <p
                    className="text-xs text-muted-foreground/70"
                    data-settings-subcopy
                  >
                    {t("local-archive.settings-card.subscribe-description")}
                  </p>
                </div>
                <Button
                  data-testid="local-archive-open-add"
                  onClick={() => setIsAddingOpen(true)}
                  size="sm"
                  variant="outline"
                >
                  {t("channels.invite.add")}
                </Button>
              </SettingsOptionRow>
            </SettingsOptionGroup>
          )}
        </div>
      </div>
    </section>
  );
}
