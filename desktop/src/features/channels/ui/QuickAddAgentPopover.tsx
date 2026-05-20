import { Check, Settings2 } from "lucide-react";
import * as React from "react";

import {
  useAcpProvidersQuery,
  useAttachManagedAgentToChannelMutation,
  useCreateChannelManagedAgentMutation,
  useManagedAgentsQuery,
  usePersonasQuery,
} from "@/features/agents/hooks";
import { useChannelMembersQuery } from "@/features/channels/hooks";
import { getActivePersonas } from "@/features/agents/lib/catalog";
import { resolvePersonaProvider } from "@/features/agents/lib/resolvePersonaProvider";
import { pickBotName } from "@/features/agents/lib/pickBotName";
import { useBotRecents } from "@/features/agents/lib/useBotRecents";
import type { AgentPersona, ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { cn } from "@/shared/lib/cn";
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
  // pickBotName always returns a string from the universal pool fallback,
  // but guard defensively in case of edge cases.
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
  // Gate channel members query to only fire when the popover is open.
  // Managed agents, personas, and providers are globally cached by
  // ChannelMembersBar (always mounted), so no extra fetch cost here.
  const managedAgentsQuery = useManagedAgentsQuery();
  const personasQuery = usePersonasQuery();
  const providersQuery = useAcpProvidersQuery();
  const membersQuery = useChannelMembersQuery(
    channelId,
    open && channelId !== null,
  );
  const attachMutation = useAttachManagedAgentToChannelMutation(channelId);
  const createMutation = useCreateChannelManagedAgentMutation(channelId);
  const { recentIds, pushRecent } = useBotRecents();

  const [pendingKey, setPendingKey] = React.useState<string | null>(null);
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);

  const managedAgents = managedAgentsQuery.data ?? [];
  const personas = React.useMemo(
    () => getActivePersonas(personasQuery.data ?? []),
    [personasQuery.data],
  );
  const providers = providersQuery.data ?? [];
  const defaultProvider = providers[0] ?? null;
  const members = membersQuery.data ?? [];

  const channelMemberPubkeys = React.useMemo(
    () => new Set(members.map((m) => normalizePubkey(m.pubkey))),
    [members],
  );

  // Build the sorted item list
  const items: QuickAddAgentItem[] = React.useMemo(() => {
    const result: QuickAddAgentItem[] = [];

    // Running agents not in this channel
    const runningAvailable = managedAgents.filter(
      (agent) =>
        (agent.status === "running" || agent.status === "deployed") &&
        !channelMemberPubkeys.has(normalizePubkey(agent.pubkey)),
    );

    // Running agents already in this channel
    const runningInChannel = managedAgents.filter(
      (agent) =>
        (agent.status === "running" || agent.status === "deployed") &&
        channelMemberPubkeys.has(normalizePubkey(agent.pubkey)),
    );

    // Personas whose agent is already in the channel (exclude from "available" list)
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

    // Sort running-available by recency
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

    // Sort available personas: recents first, then alphabetical
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
    }
  }, [open]);

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

  function handleItemClick(item: QuickAddAgentItem) {
    if (item.kind === "running-in-channel") return;
    if (pendingKey) return;
    if (!channelId) return;

    if (item.kind === "running-available") {
      void handleAddRunningAgent(item.agent);
    } else {
      void handleAddPersona(item.persona);
    }
  }

  const isLoading =
    managedAgentsQuery.isLoading ||
    personasQuery.isLoading ||
    providersQuery.isLoading;

  // Don't render the popover content when there's no channel
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
          className="flex max-h-80 flex-col"
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
          <div className="border-b px-3 py-2">
            <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
              Add agent
            </h3>
          </div>

          <div className="flex-1 overflow-y-auto">
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
                  const isItemPending = pendingKey === itemKey;

                  return (
                    <button
                      aria-selected={isInChannel}
                      className={cn(
                        "flex w-full items-center gap-2.5 px-3 py-1.5 text-left text-sm transition-colors",
                        isInChannel
                          ? "cursor-default opacity-50"
                          : "cursor-pointer hover:bg-accent focus-visible:bg-accent focus-visible:outline-none",
                        isItemPending && "pointer-events-none opacity-60",
                      )}
                      data-quick-add-item
                      disabled={isInChannel || Boolean(pendingKey)}
                      key={itemKey}
                      onClick={() => handleItemClick(item)}
                      role="option"
                      tabIndex={isInChannel ? -1 : 0}
                      type="button"
                    >
                      <QuickAddAgentAvatar
                        avatarUrl={item.avatarUrl}
                        label={item.label}
                        isRunning={item.kind !== "persona"}
                      />
                      <span className="flex-1 truncate font-medium">
                        {item.label}
                      </span>
                      {isInChannel ? (
                        <Check className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                      ) : item.kind === "running-available" ? (
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
