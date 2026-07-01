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
import {
  CHANNEL_AUX_EVENT_KINDS,
  CHANNEL_EVENT_KINDS,
  CHANNEL_MESSAGE_EVENT_KINDS,
  KIND_STREAM_MESSAGE_DIFF,
} from "@/shared/constants/kinds";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import {
  SettingsOptionGroup,
  SettingsOptionRow,
} from "@/features/settings/ui/SettingsOptionGroup";
import { SettingsSectionHeader } from "@/features/settings/ui/SettingsSectionHeader";

// ── Kind presets (derived from shared constants — never raw literals) ─────────

/** Visible message content: CHANNEL_MESSAGE_EVENT_KINDS + diff rows (own timeline row). */
const KINDS_MESSAGES: readonly number[] = [
  ...CHANNEL_MESSAGE_EVENT_KINDS,
  KIND_STREAM_MESSAGE_DIFF,
] as const;

/** Auxiliary overlay events: reactions, edits, deletions. */
const KINDS_AUX: readonly number[] = CHANNEL_AUX_EVENT_KINDS;

/** Everything the channel timeline can show. */
const KINDS_ALL: readonly number[] = CHANNEL_EVENT_KINDS;

type KindPreset = "messages" | "aux" | "all";

const PRESET_LABELS: Record<KindPreset, string> = {
  messages: "Messages & posts",
  aux: "Reactions, edits & deletions",
  all: "All channel activity",
};

function kindsForPreset(preset: KindPreset): number[] {
  switch (preset) {
    case "messages":
      return [...KINDS_MESSAGES];
    case "aux":
      return [...KINDS_AUX];
    case "all":
      return [...KINDS_ALL];
  }
}

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

// ── Main component ────────────────────────────────────────────────────────────

export function LocalArchiveSettingsCard() {
  const identityQuery = useIdentityQuery();
  const channelsQuery = useChannelsQuery();
  const [subs, setSubs] = React.useState<SaveSubscription[]>([]);
  const [isLoading, setIsLoading] = React.useState(true);
  const [deletingKey, setDeletingKey] = React.useState<string | null>(null);

  // Add-subscription form state
  const [addMode, setAddMode] = React.useState<"channel_h" | "owner_p" | null>(
    null,
  );
  const [selectedChannelId, setSelectedChannelId] = React.useState("");
  const [selectedPresets, setSelectedPresets] = React.useState<Set<KindPreset>>(
    new Set(["messages"]),
  );
  const [isAdding, setIsAdding] = React.useState(false);

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

  const handleAdd = React.useCallback(async () => {
    if (addMode === null) return;
    const pubkey = identityQuery.data?.pubkey ?? "";
    const scopeValue = addMode === "channel_h" ? selectedChannelId : pubkey;
    if (addMode === "channel_h" && !scopeValue) {
      toast.error("Select a channel first.");
      return;
    }
    if (!scopeValue) return;
    // owner_p always archives kind 24200 (agent observer frames); no preset
    // selection needed.
    let uniqueKinds: number[];
    if (addMode === "owner_p") {
      uniqueKinds = [24200];
    } else {
      const kinds = [...selectedPresets].flatMap(kindsForPreset);
      uniqueKinds = [...new Set(kinds)].sort((a, b) => a - b);
      if (uniqueKinds.length === 0) {
        toast.error("Select at least one event type.");
        return;
      }
    }
    setIsAdding(true);
    try {
      await createSaveSubscription(addMode, scopeValue, uniqueKinds);
      await reload();
      setAddMode(null);
      setSelectedChannelId("");
      setSelectedPresets(new Set(["messages"]));
      toast.success("Archive subscription created.");
    } catch (err) {
      toast.error(
        err instanceof Error ? err.message : "Failed to create subscription.",
      );
    } finally {
      setIsAdding(false);
    }
  }, [addMode, identityQuery.data, reload, selectedChannelId, selectedPresets]);

  const togglePreset = React.useCallback((preset: KindPreset) => {
    setSelectedPresets((prev) => {
      const next = new Set(prev);
      if (next.has(preset)) {
        next.delete(preset);
      } else {
        next.add(preset);
      }
      return next;
    });
  }, []);

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
                        {sub.scopeType === "owner_p"
                          ? "owner_p"
                          : sub.scopeType}{" "}
                        · kinds: {kindSummary(sub.kinds)}
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

          {addMode === null ? (
            <SettingsOptionGroup>
              <SettingsOptionRow>
                <div className="min-w-0 flex-1">
                  <p className="text-sm font-medium">Channel messages</p>
                  <p className="text-xs text-muted-foreground">
                    Archive events from a channel you're a member of.
                  </p>
                </div>
                <Button
                  data-testid="local-archive-add-channel"
                  onClick={() => {
                    setAddMode("channel_h");
                    setSelectedPresets(new Set(["messages"]));
                  }}
                  variant="outline"
                  size="sm"
                >
                  Add
                </Button>
              </SettingsOptionRow>
              <SettingsOptionRow>
                <div className="min-w-0 flex-1">
                  <p className="text-sm font-medium">My agent session frames</p>
                  <p className="text-xs text-muted-foreground">
                    Archive kind 24200 observer frames addressed to your pubkey.
                  </p>
                </div>
                <Button
                  data-testid="local-archive-add-owner"
                  disabled={ownerPAlreadySubscribed}
                  onClick={() => {
                    setAddMode("owner_p");
                    setSelectedPresets(new Set());
                  }}
                  variant="outline"
                  size="sm"
                >
                  {ownerPAlreadySubscribed ? "Active" : "Add"}
                </Button>
              </SettingsOptionRow>
            </SettingsOptionGroup>
          ) : (
            <SettingsOptionGroup>
              <div className="space-y-4 px-4 py-4">
                {addMode === "channel_h" ? (
                  <>
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
                        {joinedChannels.map((ch) => (
                          <option key={ch.id} value={ch.id}>
                            {ch.name}
                          </option>
                        ))}
                      </select>
                    </div>

                    <div>
                      <p className="mb-2 text-sm font-medium">Event types</p>
                      <div className="space-y-2">
                        {(Object.keys(PRESET_LABELS) as KindPreset[]).map(
                          (preset) => (
                            <div
                              key={preset}
                              className="flex cursor-pointer items-center gap-2 text-sm"
                            >
                              <Checkbox
                                checked={selectedPresets.has(preset)}
                                data-testid={`local-archive-preset-${preset}`}
                                id={`local-archive-preset-${preset}`}
                                onCheckedChange={() => togglePreset(preset)}
                              />
                              <label
                                className="cursor-pointer"
                                htmlFor={`local-archive-preset-${preset}`}
                              >
                                {PRESET_LABELS[preset]}
                              </label>
                            </div>
                          ),
                        )}
                      </div>
                    </div>
                  </>
                ) : (
                  <p className="text-sm text-muted-foreground">
                    Archives all kind 24200 observer frames addressed to your
                    pubkey. Kinds are fixed to{" "}
                    <code className="text-xs">[24200]</code>.
                  </p>
                )}

                <div className="flex justify-end gap-2">
                  <Button
                    disabled={isAdding}
                    onClick={() => {
                      setAddMode(null);
                      setSelectedChannelId("");
                      setSelectedPresets(new Set(["messages"]));
                    }}
                    type="button"
                    variant="outline"
                  >
                    Cancel
                  </Button>
                  <Button
                    data-testid="local-archive-confirm-add"
                    disabled={
                      isAdding ||
                      (addMode === "channel_h" && !selectedChannelId) ||
                      (addMode === "channel_h" && selectedPresets.size === 0)
                    }
                    onClick={() => void handleAdd()}
                    type="button"
                  >
                    {isAdding ? "Saving…" : "Save"}
                  </Button>
                </div>
              </div>
            </SettingsOptionGroup>
          )}
        </div>
      </div>
    </section>
  );
}
