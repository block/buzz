import { Archive, Trash2 } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import {
  createSaveSubscription,
  deleteSaveSubscription,
  listSaveSubscriptions,
  type SaveSubscription,
  type ScopeType,
} from "@/shared/api/tauriArchive";
import { KIND_AGENT_OBSERVER_FRAME } from "@/shared/constants/kinds";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import {
  SettingsOptionGroup,
  SettingsOptionRow,
} from "@/features/settings/ui/SettingsOptionGroup";
import { SettingsSectionHeader } from "@/features/settings/ui/SettingsSectionHeader";

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
    return "My agent session frames";
  }
  return sub.scopeValue;
}

function kindSummary(kinds: number[]): string {
  if (kinds.length === 0) return "no kinds";
  if (kinds.length <= 4) return kinds.join(", ");
  return `${kinds.slice(0, 3).join(", ")} +${kinds.length - 3} more`;
}

// ── Source selector (Step 1) ──────────────────────────────────────────────────

type AddSource = "channel_h" | "owner_p";

type SourceSelectorProps = {
  ownerPAlreadySubscribed: boolean;
  onSelect: (source: AddSource) => void;
};

function SourceSelector({
  ownerPAlreadySubscribed,
  onSelect,
}: SourceSelectorProps) {
  return (
    <SettingsOptionGroup>
      <SettingsOptionRow>
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium">Channel</p>
          <p className="text-xs text-muted-foreground">
            Archive events from a channel you're a member of. Access is verified
            against the relay at subscribe and persist time.
          </p>
        </div>
        <Button
          data-testid="local-archive-add-channel"
          onClick={() => onSelect("channel_h")}
          size="sm"
          variant="outline"
        >
          Add
        </Button>
      </SettingsOptionRow>
      <SettingsOptionRow>
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium">My agents' observer feed</p>
          <p className="text-xs text-muted-foreground">
            Archive kind {KIND_AGENT_OBSERVER_FRAME} observer frames addressed
            to your pubkey. Routed by pubkey, not stored by the relay.
          </p>
        </div>
        <Button
          data-testid="local-archive-add-owner"
          disabled={ownerPAlreadySubscribed}
          onClick={() => onSelect("owner_p")}
          size="sm"
          variant="outline"
        >
          {ownerPAlreadySubscribed ? "Active" : "Add"}
        </Button>
      </SettingsOptionRow>
    </SettingsOptionGroup>
  );
}

// ── Kind checklist (Step 2, channel_h only) ───────────────────────────────────

type KindChecklistProps = {
  checkedKinds: ReadonlySet<number>;
  onChange: (next: Set<number>) => void;
};

function KindChecklist({ checkedKinds, onChange }: KindChecklistProps) {
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
                {group.label}
              </label>
            </div>
            {/* Individual kind checkboxes */}
            <div className="ml-6 space-y-1.5">
              {group.items.map(({ kind, label }) => (
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
                    {label}
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
  const { invalid } = parseCustomKinds(value);
  const hasInvalid = invalid.length > 0;
  return (
    <div>
      <label
        className="mb-1.5 block text-sm font-medium"
        htmlFor="local-archive-custom-kinds"
      >
        Advanced: custom kinds
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
      <p className="mt-1 text-xs text-muted-foreground">
        Space- or comma-separated non-negative integers. Kinds already in the
        checklist above are ignored.
      </p>
      {hasInvalid && (
        <p
          className="mt-1 text-xs text-destructive"
          data-testid="local-archive-custom-kinds-error"
        >
          Invalid tokens (ignored):{" "}
          {invalid.map((t, i) => (
            <React.Fragment key={t}>
              {i > 0 && ", "}
              <code className="font-mono">{t}</code>
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
  ownerPAlreadySubscribed: boolean;
  onSaved: () => void;
  onCancel: () => void;
  pubkey: string;
};

function AddSubscriptionForm({
  channels,
  ownerPAlreadySubscribed,
  onSaved,
  onCancel,
  pubkey,
}: AddFormProps) {
  const [source, setSource] = React.useState<AddSource | null>(null);
  const [selectedChannelId, setSelectedChannelId] = React.useState("");
  const [checkedKinds, setCheckedKinds] = React.useState<Set<number>>(
    new Set(),
  );
  const [customKindsRaw, setCustomKindsRaw] = React.useState("");
  const [isAdding, setIsAdding] = React.useState(false);

  const { valid: customKinds } = parseCustomKinds(customKindsRaw);
  // `request` is non-null only when the subscription is valid to submit.
  // `canAdd` mirrors the same check for the disabled prop without recomputing.
  const request =
    source !== null
      ? buildSubscriptionRequest(
          source,
          source === "channel_h" ? selectedChannelId : pubkey,
          checkedKinds,
          customKinds,
        )
      : null;
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
      toast.success("Archive subscription created.");
    } catch (err) {
      toast.error(
        err instanceof Error ? err.message : "Failed to create subscription.",
      );
    } finally {
      setIsAdding(false);
    }
  }, [request, onSaved]);

  const handleCancel = () => {
    setSource(null);
    setSelectedChannelId("");
    setCheckedKinds(new Set());
    setCustomKindsRaw("");
    onCancel();
  };

  // Step 1: source picker
  if (source === null) {
    return (
      <SourceSelector
        ownerPAlreadySubscribed={ownerPAlreadySubscribed}
        onSelect={setSource}
      />
    );
  }

  // Step 2: event types + confirm
  return (
    <SettingsOptionGroup>
      <div className="space-y-5 px-4 py-4">
        {/* Step 1 summary + back link */}
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium">
            {source === "channel_h" ? "Channel" : "My agents' observer feed"}
          </span>
          <button
            className="text-xs text-muted-foreground underline-offset-2 hover:underline"
            data-testid="local-archive-back"
            onClick={() => setSource(null)}
            type="button"
          >
            Change source
          </button>
        </div>

        {source === "channel_h" ? (
          <>
            {/* Channel picker */}
            <div>
              <label
                className="mb-1.5 block text-sm font-medium"
                htmlFor="local-archive-channel-select"
              >
                Channel
              </label>
              <select
                className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                data-testid="local-archive-channel-select"
                id="local-archive-channel-select"
                onChange={(e) => setSelectedChannelId(e.target.value)}
                value={selectedChannelId}
              >
                <option value="">Select a channel…</option>
                {channels.map((ch) => (
                  <option key={ch.id} value={ch.id}>
                    {ch.name}
                  </option>
                ))}
              </select>
            </div>

            {/* Event types (per-kind checklist) */}
            <div>
              <p className="mb-3 text-sm font-medium">Event types</p>
              <KindChecklist
                checkedKinds={checkedKinds}
                onChange={setCheckedKinds}
              />
            </div>

            {/* Advanced: custom kinds */}
            <CustomKindsInput
              onChange={setCustomKindsRaw}
              value={customKindsRaw}
            />
          </>
        ) : (
          /* owner_p: fixed / informational */
          <p className="text-sm text-muted-foreground">
            Archives all kind {KIND_AGENT_OBSERVER_FRAME} observer frames
            addressed to your pubkey. The event type is fixed to{" "}
            <code className="text-xs">[{KIND_AGENT_OBSERVER_FRAME}]</code> —
            these frames are never stored by the relay and cannot be filtered by
            channel at this layer.
          </p>
        )}

        <div className="flex justify-end gap-2">
          <Button
            disabled={isAdding}
            onClick={handleCancel}
            type="button"
            variant="outline"
          >
            Cancel
          </Button>
          <Button
            data-testid="local-archive-confirm-add"
            disabled={isAdding || !canAdd}
            onClick={() => void handleAdd()}
            type="button"
          >
            {isAdding ? "Saving…" : "Save"}
          </Button>
        </div>
      </div>
    </SettingsOptionGroup>
  );
}

// ── Main component ────────────────────────────────────────────────────────────

export function LocalArchiveSettingsCard() {
  const identityQuery = useIdentityQuery();
  const channelsQuery = useChannelsQuery();
  const [subs, setSubs] = React.useState<SaveSubscription[]>([]);
  const [isLoading, setIsLoading] = React.useState(true);
  const [deletingKey, setDeletingKey] = React.useState<string | null>(null);
  const [isAddingOpen, setIsAddingOpen] = React.useState(false);

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
        toast.success("Archive subscription removed.");
      } catch (err) {
        toast.error(
          err instanceof Error ? err.message : "Failed to remove subscription.",
        );
      } finally {
        setDeletingKey(null);
      }
    },
    [reload],
  );

  const ownerPAlreadySubscribed = subs.some((s) => s.scopeType === "owner_p");

  return (
    <section className="min-w-0" data-testid="settings-local-archive">
      <SettingsSectionHeader
        title="Local Archive"
        description="Save copies of relay messages to a local SQLite database in your Buzz nest. Events are re-verified against the relay at archive time."
      />

      <div className="space-y-6">
        {/* Existing subscriptions */}
        <div className="space-y-3" data-testid="local-archive-subscriptions">
          <h3 className="text-sm font-medium">
            Active subscriptions
            {subs.length > 0 ? ` (${subs.length})` : ""}
          </h3>
          {isLoading ? (
            <SettingsOptionGroup>
              <div className="px-4 py-3 text-sm font-normal text-muted-foreground">
                Loading…
              </div>
            </SettingsOptionGroup>
          ) : subs.length === 0 ? (
            <SettingsOptionGroup>
              <div className="px-4 py-3 text-sm font-normal text-muted-foreground">
                No subscriptions yet. Add one below.
              </div>
            </SettingsOptionGroup>
          ) : (
            <SettingsOptionGroup>
              {subs.map((sub) => {
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
                      <p className="text-xs text-muted-foreground">
                        {sub.scopeType} · kinds: {kindSummary(sub.kinds)}
                      </p>
                    </div>
                    <Button
                      aria-label={`Remove archive subscription for ${scopeLabel(sub, channelNameById)}`}
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

        {/* Add subscription */}
        <div className="space-y-3" data-testid="local-archive-add">
          <h3 className="text-sm font-medium">Add subscription</h3>
          {isAddingOpen ? (
            <AddSubscriptionForm
              channels={joinedChannels}
              onCancel={() => setIsAddingOpen(false)}
              onSaved={() => {
                setIsAddingOpen(false);
                void reload();
              }}
              ownerPAlreadySubscribed={ownerPAlreadySubscribed}
              pubkey={identityQuery.data?.pubkey ?? ""}
            />
          ) : (
            <SettingsOptionGroup>
              <SettingsOptionRow>
                <div className="min-w-0 flex-1">
                  <p className="text-sm font-medium">
                    Subscribe to a channel or observer feed
                  </p>
                  <p className="text-xs text-muted-foreground">
                    Choose a source and select which event types to archive.
                  </p>
                </div>
                <Button
                  data-testid="local-archive-open-add"
                  onClick={() => setIsAddingOpen(true)}
                  size="sm"
                  variant="outline"
                >
                  Add
                </Button>
              </SettingsOptionRow>
            </SettingsOptionGroup>
          )}
        </div>
      </div>
    </section>
  );
}
