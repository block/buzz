import { Check, Settings2 } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import * as React from "react";

import {
  useAcpProvidersQuery,
  useAttachManagedAgentToChannelMutation,
  useCreateChannelManagedAgentMutation,
  useCreateChannelManagedAgentsMutation,
  useManagedAgentsQuery,
  usePersonasQuery,
  useTeamsQuery,
} from "@/features/agents/hooks";
import { Toggle } from "@/shared/ui/toggle";
import { useChannelMembersQuery } from "@/features/channels/hooks";
import { getActivePersonas } from "@/features/agents/lib/catalog";
import { resolvePersonaProvider } from "@/features/agents/lib/resolvePersonaProvider";
import { pickBotName } from "@/features/agents/lib/pickBotName";
import { useBotRecents } from "@/features/agents/lib/useBotRecents";
import {
  getUsableTeams,
  resolveTeamPersonas,
} from "@/features/agents/lib/teamPersonas";
import type { AgentPersona, AgentTeam, ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Spinner } from "@/shared/ui/spinner";

// ── Types ─────────────────────────────────────────────────────────────────────

type RunningAvailableItem = {
  kind: "running-available";
  agent: ManagedAgent;
  persona: AgentPersona | null;
  label: string;
  avatarUrl: string | null;
};

type RunningInChannelItem = {
  kind: "running-in-channel";
  agent: ManagedAgent;
  persona: AgentPersona | null;
  label: string;
  avatarUrl: string | null;
};

type PersonaItem = {
  kind: "persona";
  persona: AgentPersona;
  label: string;
  avatarUrl: string | null;
};

type QuickAddAgentItem =
  | RunningAvailableItem
  | RunningInChannelItem
  | PersonaItem;

type QuickAddAgentPopoverProps = {
  channelId: string | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onMoreOptions: () => void;
  children: React.ReactNode;
};

// ── Helpers ───────────────────────────────────────────────────────────────────

function getItemKey(item: QuickAddAgentItem): string {
  switch (item.kind) {
    case "persona":
      return `persona:${item.persona.id}`;
    case "running-available":
    case "running-in-channel":
      return `agent:${item.agent.pubkey}`;
  }
}

function safeBotName(persona: AgentPersona, usedNames: Set<string>): string {
  const pool = persona.namePool ?? [];
  const name = pickBotName(pool, usedNames);
  if (name && name.trim().length > 0) return name;
  return persona.displayName || "Agent";
}

// ── Component ─────────────────────────────────────────────────────────────────

export function QuickAddAgentPopover({
  channelId,
  open,
  onOpenChange,
  onMoreOptions,
  children,
}: QuickAddAgentPopoverProps) {
  const managedAgentsQuery = useManagedAgentsQuery();
  const personasQuery = usePersonasQuery();
  const providersQuery = useAcpProvidersQuery();
  const teamsQuery = useTeamsQuery();
  const membersQuery = useChannelMembersQuery(
    channelId,
    open && channelId !== null,
  );
  const attachMutation = useAttachManagedAgentToChannelMutation(channelId);
  const createMutation = useCreateChannelManagedAgentMutation(channelId);
  const batchCreateMutation = useCreateChannelManagedAgentsMutation(channelId);
  const { recentIds, pushRecent } = useBotRecents();

  const [pendingKey, setPendingKey] = React.useState<string | null>(null);
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);
  const [selectMode, setSelectMode] = React.useState(false);
  const [selectedKeys, setSelectedKeys] = React.useState<Set<string>>(
    new Set(),
  );
  const [selectedTeamIds, setSelectedTeamIds] = React.useState<Set<string>>(
    new Set(),
  );

  const managedAgents = managedAgentsQuery.data ?? [];
  const personas = React.useMemo(
    () => getActivePersonas(personasQuery.data ?? []),
    [personasQuery.data],
  );
  const providers = providersQuery.data ?? [];
  const defaultProvider = providers[0] ?? null;
  const members = membersQuery.data ?? [];
  const teams = teamsQuery.data ?? [];

  const channelMemberPubkeys = React.useMemo(
    () => new Set(members.map((m) => normalizePubkey(m.pubkey))),
    [members],
  );

  const usableTeams = React.useMemo(
    () => getUsableTeams(teams, personas),
    [teams, personas],
  );

  // Build the sorted item list
  const items: QuickAddAgentItem[] = React.useMemo(() => {
    const result: QuickAddAgentItem[] = [];

    const runningAvailable = managedAgents.filter(
      (agent) =>
        (agent.status === "running" || agent.status === "deployed") &&
        !channelMemberPubkeys.has(normalizePubkey(agent.pubkey)),
    );

    const runningInChannel = managedAgents.filter(
      (agent) =>
        (agent.status === "running" || agent.status === "deployed") &&
        channelMemberPubkeys.has(normalizePubkey(agent.pubkey)),
    );

    const personaIdsInChannel = new Set(
      managedAgents
        .filter((agent) =>
          channelMemberPubkeys.has(normalizePubkey(agent.pubkey)),
        )
        .map((agent) => agent.personaId)
        .filter((id): id is string => Boolean(id)),
    );

    const availablePersonas = personas.filter(
      (persona) =>
        !personaIdsInChannel.has(persona.id) &&
        !runningAvailable.some((agent) => agent.personaId === persona.id),
    );

    const sortedRunningAvailable = [...runningAvailable].sort((a, b) => {
      const aPersonaIdx = a.personaId ? recentIds.indexOf(a.personaId) : -1;
      const bPersonaIdx = b.personaId ? recentIds.indexOf(b.personaId) : -1;
      const aScore = aPersonaIdx >= 0 ? aPersonaIdx : 999;
      const bScore = bPersonaIdx >= 0 ? bPersonaIdx : 999;
      return aScore - bScore;
    });

    for (const agent of sortedRunningAvailable) {
      const persona = agent.personaId
        ? (personas.find((p) => p.id === agent.personaId) ?? null)
        : null;
      result.push({
        kind: "running-available",
        agent,
        persona,
        label: agent.name,
        avatarUrl: persona?.avatarUrl ?? null,
      });
    }

    for (const agent of runningInChannel) {
      const persona = agent.personaId
        ? (personas.find((p) => p.id === agent.personaId) ?? null)
        : null;
      result.push({
        kind: "running-in-channel",
        agent,
        persona,
        label: agent.name,
        avatarUrl: persona?.avatarUrl ?? null,
      });
    }

    const sortedPersonas = [...availablePersonas].sort((a, b) => {
      const aIdx = recentIds.indexOf(a.id);
      const bIdx = recentIds.indexOf(b.id);
      if (aIdx >= 0 && bIdx >= 0) return aIdx - bIdx;
      if (aIdx >= 0) return -1;
      if (bIdx >= 0) return 1;
      return a.displayName.localeCompare(b.displayName);
    });

    for (const persona of sortedPersonas) {
      result.push({
        kind: "persona",
        persona,
        label: persona.displayName,
        avatarUrl: persona.avatarUrl,
      });
    }

    return result;
  }, [managedAgents, personas, channelMemberPubkeys, recentIds]);

  // Reset state when popover closes
  React.useEffect(() => {
    if (!open) {
      setPendingKey(null);
      setErrorMessage(null);
      setSelectMode(false);
      setSelectedKeys(new Set());
      setSelectedTeamIds(new Set());
    }
  }, [open]);

  // ── Single-add handlers ───────────────────────────────────────────────────

  async function handleAddRunningAgent(agent: ManagedAgent) {
    if (!channelId) return;
    const key = `agent:${agent.pubkey}`;
    setPendingKey(key);
    setErrorMessage(null);

    try {
      await attachMutation.mutateAsync({ agent, ensureRunning: true });
      if (agent.personaId) pushRecent(agent.personaId);
      onOpenChange(false);
    } catch (err) {
      setErrorMessage(
        err instanceof Error ? err.message : "Failed to add agent.",
      );
      setPendingKey(null);
    }
  }

  async function handleAddPersona(persona: AgentPersona) {
    if (!channelId) return;
    const key = `persona:${persona.id}`;
    setPendingKey(key);
    setErrorMessage(null);

    const { provider } = resolvePersonaProvider(
      persona.provider,
      providers,
      defaultProvider,
    );

    if (!provider) {
      setErrorMessage("No agent runtime available.");
      setPendingKey(null);
      return;
    }

    const usedNames = new Set(managedAgents.map((a) => a.name));
    const instanceName = safeBotName(persona, usedNames);

    try {
      await createMutation.mutateAsync({
        provider,
        name: instanceName,
        systemPrompt: persona.systemPrompt,
        avatarUrl: persona.avatarUrl ?? undefined,
        personaId: persona.id,
        model: persona.model ?? undefined,
      });
      pushRecent(persona.id);
      onOpenChange(false);
    } catch (err) {
      setErrorMessage(
        err instanceof Error ? err.message : "Failed to add agent.",
      );
      setPendingKey(null);
    }
  }

  // ── Multi-select handlers ─────────────────────────────────────────────────

  function handleCancelSelect() {
    setSelectMode(false);
    setSelectedKeys(new Set());
    setSelectedTeamIds(new Set());
  }

  function toggleSelection(key: string) {
    setSelectedKeys((prev) => {
      const next = new Set(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      return next;
    });
  }

  function handleTeamToggle(team: AgentTeam, pressed: boolean) {
    const resolution = resolveTeamPersonas(team, personas);
    const memberKeys: string[] = [];
    for (const persona of resolution.resolvedPersonas) {
      const runningItem = items.find(
        (i) =>
          i.kind === "running-available" && i.agent.personaId === persona.id,
      );
      if (runningItem) {
        memberKeys.push(getItemKey(runningItem));
      } else {
        const personaItem = items.find(
          (i) => i.kind === "persona" && i.persona.id === persona.id,
        );
        if (personaItem) {
          memberKeys.push(getItemKey(personaItem));
        }
      }
    }

    setSelectedTeamIds((prev) => {
      const next = new Set(prev);
      if (pressed) {
        next.add(team.id);
      } else {
        next.delete(team.id);
      }
      return next;
    });

    setSelectedKeys((prev) => {
      const next = new Set(prev);
      if (pressed) {
        for (const key of memberKeys) {
          next.add(key);
        }
      } else {
        for (const key of memberKeys) {
          next.delete(key);
        }
      }
      return next;
    });
  }

  async function handleBatchAdd() {
    if (!channelId || selectedKeys.size === 0) return;
    setPendingKey("batch");
    setErrorMessage(null);

    const usedNames = new Set(managedAgents.map((a) => a.name));
    const toAttach: ManagedAgent[] = [];
    const toCreate: Array<{ persona: AgentPersona; instanceName: string }> = [];

    for (const key of selectedKeys) {
      const item = items.find((i) => getItemKey(i) === key);
      if (!item || item.kind === "running-in-channel") continue;

      if (item.kind === "running-available") {
        toAttach.push(item.agent);
      } else {
        const instanceName = safeBotName(item.persona, usedNames);
        usedNames.add(instanceName);
        toCreate.push({ persona: item.persona, instanceName });
      }
    }

    try {
      for (const agent of toAttach) {
        await attachMutation.mutateAsync({ agent, ensureRunning: true });
        if (agent.personaId) pushRecent(agent.personaId);
      }

      if (toCreate.length > 0 && defaultProvider) {
        const inputs = toCreate.map(({ persona, instanceName }) => {
          const { provider } = resolvePersonaProvider(
            persona.provider,
            providers,
            defaultProvider,
          );
          const providerToUse = provider ?? defaultProvider;
          return {
            provider: {
              id: providerToUse.id,
              label: providerToUse.label,
              command: providerToUse.command,
              defaultArgs: providerToUse.defaultArgs,
              mcpCommand: providerToUse.mcpCommand,
            },
            name: instanceName,
            systemPrompt: persona.systemPrompt,
            avatarUrl: persona.avatarUrl ?? undefined,
            personaId: persona.id,
            model: persona.model ?? undefined,
          };
        });

        await batchCreateMutation.mutateAsync(inputs);
        for (const { persona } of toCreate) {
          pushRecent(persona.id);
        }
      }

      onOpenChange(false);
    } catch (err) {
      setErrorMessage(
        err instanceof Error ? err.message : "Failed to add agents.",
      );
      setPendingKey(null);
    }
  }

  // ── Item click dispatcher ─────────────────────────────────────────────────

  function handleItemClick(item: QuickAddAgentItem) {
    if (item.kind === "running-in-channel") return;
    if (pendingKey) return;
    if (!channelId) return;

    if (selectMode) {
      toggleSelection(getItemKey(item));
    } else {
      if (item.kind === "running-available") {
        void handleAddRunningAgent(item.agent);
      } else {
        void handleAddPersona(item.persona);
      }
    }
  }

  const isLoading =
    managedAgentsQuery.isLoading ||
    personasQuery.isLoading ||
    providersQuery.isLoading;

  const multiSelectActive = selectMode && selectedKeys.size > 0;

  if (!channelId) {
    return <>{children}</>;
  }

  return (
    <Popover open={open} onOpenChange={onOpenChange}>
      <PopoverTrigger asChild>{children}</PopoverTrigger>
      <PopoverContent
        align="end"
        className="w-72 overflow-hidden p-0"
        sideOffset={6}
      >
        <div
          className="flex flex-col"
          role="menu"
          onKeyDown={(e) => {
            const container = e.currentTarget;
            const buttons = Array.from(
              container.querySelectorAll<HTMLButtonElement>(
                "[data-quick-add-item]:not([disabled])",
              ),
            );
            if (buttons.length === 0) return;
            const focused = document.activeElement as HTMLElement | null;
            const currentIdx = focused
              ? buttons.indexOf(focused as HTMLButtonElement)
              : -1;

            if (e.key === "ArrowDown") {
              e.preventDefault();
              const next = currentIdx < buttons.length - 1 ? currentIdx + 1 : 0;
              buttons[next]?.focus();
            } else if (e.key === "ArrowUp") {
              e.preventDefault();
              const prev = currentIdx > 0 ? currentIdx - 1 : buttons.length - 1;
              buttons[prev]?.focus();
            }
          }}
        >
          {/* Header with animated title / team toggles */}
          <div className="relative flex min-h-10 items-center gap-2 border-b pl-3 pr-1.5 py-1.5">
            {/* Title — always visible */}
            <h3 className="shrink-0 text-sm font-semibold text-foreground">
              Add agent{selectMode ? (
                <motion.span
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.3 }}
                >
                  s
                </motion.span>
              ) : null}
            </h3>

            {/* Team toggles — appear to the right of title */}
            <AnimatePresence>
              {selectMode ? (
                <motion.div
                  key="team-chips"
                  className="relative flex min-w-0 flex-1 items-center"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.15 }}
                >
                  <div className="flex items-center gap-1.5 overflow-x-auto">
                    {usableTeams.map((team, index) => (
                      <motion.div
                        key={team.id}
                        initial={{ opacity: 0, x: index === 0 ? 0 : 12 }}
                        animate={{ opacity: 1, x: 0 }}
                        transition={{
                          duration: 0.2,
                          delay: index * 0.05,
                        }}
                        className="shrink-0"
                      >
                        <Toggle
                          className="h-8 rounded-full px-3 text-xs"
                          onPressedChange={(pressed) =>
                            handleTeamToggle(team, pressed)
                          }
                          pressed={selectedTeamIds.has(team.id)}
                          size="sm"
                          variant="outline"
                        >
                          {team.name}
                        </Toggle>
                      </motion.div>
                    ))}
                  </div>
                  {/* Gradient fade on right edge — scroll affordance */}
                  <div className="pointer-events-none absolute right-0 top-0 h-full w-6 bg-gradient-to-l from-popover to-transparent" />
                </motion.div>
              ) : null}
            </AnimatePresence>

            {/* Right side: Select / Cancel button */}
            {usableTeams.length > 0 ? (
              <div className="ml-auto flex shrink-0 items-center">
                <Button
                  className="border border-input bg-transparent"
                  onClick={() => {
                    if (selectMode) {
                      handleCancelSelect();
                    } else {
                      setSelectMode(true);
                    }
                  }}
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  {selectMode ? "Cancel" : "Select"}
                </Button>
              </div>
            ) : null}
          </div>

          {/* Scrollable content — clips mid-item to hint at more */}
          <div className="max-h-[13.75rem] flex-1 overflow-y-auto">
            {isLoading ? (
              <div className="flex items-center justify-center py-6">
                <Spinner className="h-4 w-4 text-muted-foreground" />
              </div>
            ) : items.length === 0 ? (
              <div className="px-3 py-4 text-center text-sm text-muted-foreground">
                No agents available.
              </div>
            ) : (
              <div
                aria-label="Available agents"
                className="py-1"
                role="listbox"
              >
                {items.map((item) => {
                  const itemKey = getItemKey(item);
                  const isInChannel = item.kind === "running-in-channel";
                  const isItemPending =
                    pendingKey === itemKey || pendingKey === "batch";
                  const isSelected = selectedKeys.has(itemKey);

                  return (
                    <button
                      aria-selected={isInChannel || isSelected}
                      className={cn(
                        "flex w-full items-center gap-2.5 px-3 py-1.5 text-left text-sm transition-colors",
                        isInChannel
                          ? "cursor-default opacity-50"
                          : "cursor-pointer hover:bg-accent focus-visible:bg-accent focus-visible:outline-none",
                        isItemPending && "pointer-events-none opacity-60",
                        isSelected && "bg-accent/50",
                      )}
                      data-quick-add-item
                      disabled={isInChannel || Boolean(pendingKey)}
                      key={itemKey}
                      onClick={() => handleItemClick(item)}
                      role="option"
                      tabIndex={isInChannel ? -1 : 0}
                      type="button"
                    >
                      {selectMode && !isInChannel ? (
                        <div
                          className={cn(
                            "flex h-4 w-4 shrink-0 items-center justify-center rounded border transition-colors",
                            isSelected
                              ? "border-primary bg-primary text-primary-foreground"
                              : "border-muted-foreground/40",
                          )}
                        >
                          {isSelected ? <Check className="h-3 w-3" /> : null}
                        </div>
                      ) : null}
                      <QuickAddAgentAvatar
                        avatarUrl={item.avatarUrl}
                        label={item.label}
                        isRunning={item.kind !== "persona"}
                      />
                      <span className="min-w-0 flex-1 truncate font-medium">
                        {item.label}
                      </span>
                      {isInChannel ? (
                        <Check className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                      ) : item.kind === "running-available" && !selectMode ? (
                        <span className="shrink-0 rounded-full bg-emerald-500/15 px-1.5 py-0.5 text-[10px] font-medium text-emerald-600 dark:text-emerald-400">
                          running
                        </span>
                      ) : null}
                      {isItemPending ? (
                        <Spinner className="h-3.5 w-3.5 shrink-0 text-primary" />
                      ) : null}
                    </button>
                  );
                })}
              </div>
            )}
          </div>

          {errorMessage ? (
            <div className="border-t px-3 py-2">
              <p className="text-xs text-destructive">{errorMessage}</p>
            </div>
          ) : null}

          <div className="border-t">
            <button
              className="flex w-full items-center gap-2 px-3 py-2 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              data-quick-add-item
              data-testid="quick-add-more-options"
              onClick={() => {
                onOpenChange(false);
                onMoreOptions();
              }}
              type="button"
            >
              <Settings2 className="h-3.5 w-3.5" />
              <span>More options…</span>
            </button>
          </div>

          {multiSelectActive ? (
            <div className="border-t px-3 py-2">
              <Button
                className="w-full"
                data-testid="quick-add-batch-confirm"
                disabled={Boolean(pendingKey)}
                onClick={() => void handleBatchAdd()}
                size="sm"
                type="button"
              >
                {pendingKey === "batch" ? (
                  <Spinner className="h-3 w-3" />
                ) : null}
                Add ({selectedKeys.size})
              </Button>
            </div>
          ) : null}
        </div>
      </PopoverContent>
    </Popover>
  );
}

// ── Avatar helper ─────────────────────────────────────────────────────────────

function QuickAddAgentAvatar({
  avatarUrl,
  label,
  isRunning,
}: {
  avatarUrl: string | null;
  label: string;
  isRunning: boolean;
}) {
  const initials = label
    .split(" ")
    .map((part) => part[0])
    .join("")
    .slice(0, 2)
    .toUpperCase();

  return (
    <div className="relative flex h-7 w-7 shrink-0 items-center justify-center rounded-full border border-border/50 bg-muted/30">
      {avatarUrl ? (
        <img
          alt={label}
          className="h-full w-full rounded-full object-cover"
          referrerPolicy="no-referrer"
          src={rewriteRelayUrl(avatarUrl)}
        />
      ) : (
        <span className="text-[10px] font-semibold text-muted-foreground">
          {initials}
        </span>
      )}
      {isRunning ? (
        <span className="absolute -bottom-0.5 -right-0.5 h-2.5 w-2.5 rounded-full border-2 border-background bg-emerald-500" />
      ) : null}
    </div>
  );
}
