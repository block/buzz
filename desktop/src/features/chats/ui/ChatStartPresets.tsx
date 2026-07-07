import * as React from "react";
import {
  Bot,
  Check,
  Notebook,
  NotepadTextDashed,
  Search,
  UserPlus,
  Users,
  X,
} from "lucide-react";

import type { ChatProject } from "@/features/chats/lib/chatSetup";
import { useUserSearchQuery } from "@/features/profile/hooks";
import type { AgentTeam, ManagedAgent } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Input } from "@/shared/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { UserAvatar } from "@/shared/ui/UserAvatar";

/** Which agent(s) the new chat starts with. */
export type ChatAgentPreset =
  | { kind: "default" }
  | { kind: "agent"; agent: ManagedAgent }
  | { kind: "team"; team: AgentTeam };

export type ChatInvitee = {
  pubkey: string;
  displayName: string | null;
  avatarUrl: string | null;
};

export function chatAgentPresetLabel(
  preset: ChatAgentPreset,
  defaultAgentName: string,
) {
  if (preset.kind === "agent") {
    return preset.agent.name;
  }
  if (preset.kind === "team") {
    return preset.team.name;
  }
  return defaultAgentName;
}

/**
 * Preset cards for the new-chat screen — same container language as the
 * channel-intro action cards: default agent (swap to another agent or a
 * team), the project's working directory, and pre-invited people.
 */
export function ChatStartPresets({
  agentPreset,
  agents,
  defaultAgentName,
  invited,
  onAgentPresetChange,
  onInvitedChange,
  projectCard,
  teams,
}: {
  agentPreset: ChatAgentPreset;
  agents: ManagedAgent[];
  defaultAgentName: string;
  invited: ChatInvitee[];
  onAgentPresetChange: (preset: ChatAgentPreset) => void;
  onInvitedChange: (invited: ChatInvitee[]) => void;
  /** Rendered as the middle card — the project picker owns its popover. */
  projectCard: React.ReactNode;
  teams: AgentTeam[];
}) {
  return (
    <div className="mt-6 flex flex-wrap justify-center gap-3">
      <AgentPresetCard
        agentPreset={agentPreset}
        agents={agents}
        defaultAgentName={defaultAgentName}
        onAgentPresetChange={onAgentPresetChange}
        teams={teams}
      />
      {projectCard}
      <InviteCard invited={invited} onInvitedChange={onInvitedChange} />
    </div>
  );
}

type PresetCardProps = React.ComponentPropsWithoutRef<"button"> & {
  icon: React.ReactNode;
  subtitle: string;
  testId?: string;
  title: string;
};

// Plain-prop spread + ref forwarding so Radix `asChild` triggers can drive
// the card (popover click/aria props arrive via ...props).
export function PresetCard({
  icon,
  subtitle,
  testId,
  title,
  ...props
}: PresetCardProps) {
  return (
    <button
      className="flex h-32 w-64 shrink-0 flex-col rounded-2xl border border-border/70 bg-background/70 p-4 text-left transition-colors hover:bg-muted/60 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
      data-testid={testId}
      type="button"
      {...props}
    >
      <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-muted/70 text-muted-foreground [&_svg]:h-4 [&_svg]:w-4">
        {icon}
      </span>
      <span className="mt-auto min-w-0 pt-3">
        <span className="block truncate text-base font-medium leading-6 text-foreground">
          {title}
        </span>
        <span className="block truncate text-sm leading-5 text-muted-foreground">
          {subtitle}
        </span>
      </span>
    </button>
  );
}

function PickerRow({
  checked,
  icon,
  label,
  meta,
  onSelect,
}: {
  checked?: boolean;
  icon: React.ReactNode;
  label: string;
  meta?: string | null;
  onSelect: () => void;
}) {
  return (
    <button
      className="flex w-full items-center gap-3 rounded-lg px-2 py-2 text-left text-sm outline-hidden transition-colors hover:bg-muted/60 focus-visible:bg-muted/60"
      onClick={onSelect}
      type="button"
    >
      <span className="flex h-6 w-6 shrink-0 items-center justify-center text-muted-foreground [&_svg]:h-4 [&_svg]:w-4">
        {icon}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate font-medium">{label}</span>
        {meta ? (
          <span className="block truncate text-xs text-muted-foreground">
            {meta}
          </span>
        ) : null}
      </span>
      {checked ? <Check className="h-4 w-4 shrink-0" /> : null}
    </button>
  );
}

function AgentPresetCard({
  agentPreset,
  agents,
  defaultAgentName,
  onAgentPresetChange,
  teams,
}: {
  agentPreset: ChatAgentPreset;
  agents: ManagedAgent[];
  defaultAgentName: string;
  onAgentPresetChange: (preset: ChatAgentPreset) => void;
  teams: AgentTeam[];
}) {
  const [open, setOpen] = React.useState(false);
  const selectedAgent = agentPreset.kind === "agent" ? agentPreset.agent : null;
  const icon =
    agentPreset.kind === "team" ? (
      <Users aria-hidden />
    ) : selectedAgent?.avatarUrl ? (
      <UserAvatar
        avatarUrl={selectedAgent.avatarUrl}
        displayName={selectedAgent.name}
        size="sm"
      />
    ) : (
      <Bot aria-hidden />
    );

  return (
    <Popover onOpenChange={setOpen} open={open}>
      <PopoverTrigger asChild>
        <PresetCard
          icon={icon}
          subtitle={agentPreset.kind === "team" ? "Team" : "Default agent"}
          testId="chat-preset-agent"
          title={chatAgentPresetLabel(agentPreset, defaultAgentName)}
        />
      </PopoverTrigger>
      <PopoverContent align="start" className="w-80 p-2">
        <div className="max-h-72 overflow-y-auto">
          <PickerRow
            checked={agentPreset.kind === "default"}
            icon={<Bot aria-hidden />}
            label={defaultAgentName}
            meta="Default agent"
            onSelect={() => {
              onAgentPresetChange({ kind: "default" });
              setOpen(false);
            }}
          />
          {agents.map((agent) => (
            <PickerRow
              checked={
                agentPreset.kind === "agent" &&
                normalizePubkey(agentPreset.agent.pubkey) ===
                  normalizePubkey(agent.pubkey)
              }
              icon={
                <UserAvatar
                  avatarUrl={agent.avatarUrl ?? null}
                  displayName={agent.name}
                  size="xs"
                />
              }
              key={agent.pubkey}
              label={agent.name}
              meta={
                agent.status === "running" || agent.status === "deployed"
                  ? "Running"
                  : "Stopped"
              }
              onSelect={() => {
                onAgentPresetChange({ kind: "agent", agent });
                setOpen(false);
              }}
            />
          ))}
          {teams.length > 0 ? (
            <>
              <div className="my-2 border-t border-border/60" />
              <div className="px-2 pb-1 text-xs font-medium text-muted-foreground">
                Teams
              </div>
              {teams.map((team) => (
                <PickerRow
                  checked={
                    agentPreset.kind === "team" &&
                    agentPreset.team.id === team.id
                  }
                  icon={<Users aria-hidden />}
                  key={team.id}
                  label={team.name}
                  meta={
                    team.personaIds.length === 1
                      ? "1 agent"
                      : `${team.personaIds.length} agents`
                  }
                  onSelect={() => {
                    onAgentPresetChange({ kind: "team", team });
                    setOpen(false);
                  }}
                />
              ))}
            </>
          ) : null}
        </div>
      </PopoverContent>
    </Popover>
  );
}

export function ProjectPresetCard({
  isNoProjectSelected,
  selectedProject,
  ...props
}: {
  isNoProjectSelected: boolean;
  selectedProject: ChatProject | null;
} & React.ComponentPropsWithoutRef<"button">) {
  return (
    <PresetCard
      {...props}
      icon={
        selectedProject ? (
          <Notebook aria-hidden />
        ) : (
          <NotepadTextDashed aria-hidden />
        )
      }
      subtitle={
        selectedProject
          ? (selectedProject.path ?? "No directory")
          : isNoProjectSelected
            ? "Free chat — no directory"
            : "Pick a project"
      }
      testId="chat-preset-directory"
      title={selectedProject ? selectedProject.name : "No project"}
    />
  );
}

function InviteCard({
  invited,
  onInvitedChange,
}: {
  invited: ChatInvitee[];
  onInvitedChange: (invited: ChatInvitee[]) => void;
}) {
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  // Same search path as the channel invite (MembersSidebar): shared hook,
  // normalized query, cached results.
  const deferredQuery = React.useDeferredValue(query.trim());
  const searchQuery = useUserSearchQuery(deferredQuery, {
    enabled: open && deferredQuery.length > 0,
    limit: 8,
  });
  const results = searchQuery.data ?? [];
  const isSearching = searchQuery.isFetching;

  const invitedPubkeys = new Set(
    invited.map((person) => normalizePubkey(person.pubkey)),
  );

  return (
    <Popover onOpenChange={setOpen} open={open}>
      <PopoverTrigger asChild>
        <PresetCard
          icon={<UserPlus aria-hidden />}
          subtitle={
            invited.length === 0
              ? "Add someone to the chat"
              : invited
                  .map((person) => person.displayName ?? "someone")
                  .join(", ")
          }
          testId="chat-preset-invite"
          title={invited.length === 0 ? "Invite" : `${invited.length} invited`}
        />
      </PopoverTrigger>
      <PopoverContent align="start" className="w-80 p-2">
        <div className="relative mb-2">
          <Search className="absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            className="h-9 pl-8"
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search people"
            value={query}
          />
        </div>
        {invited.length > 0 ? (
          <div className="mb-2 flex flex-col gap-1">
            {invited.map((person) => (
              <div
                className="flex items-center gap-2 rounded-lg bg-muted/40 px-2 py-1.5 text-sm"
                key={person.pubkey}
              >
                <UserAvatar
                  avatarUrl={person.avatarUrl}
                  displayName={person.displayName ?? "?"}
                  size="xs"
                />
                <span className="min-w-0 flex-1 truncate">
                  {person.displayName ?? person.pubkey.slice(0, 8)}
                </span>
                <button
                  aria-label={`Remove ${person.displayName ?? "invitee"}`}
                  className="text-muted-foreground hover:text-foreground"
                  onClick={() =>
                    onInvitedChange(
                      invited.filter(
                        (candidate) => candidate.pubkey !== person.pubkey,
                      ),
                    )
                  }
                  type="button"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              </div>
            ))}
          </div>
        ) : null}
        <div className="max-h-56 overflow-y-auto">
          {results
            .filter((user) => !invitedPubkeys.has(normalizePubkey(user.pubkey)))
            .map((user) => (
              <PickerRow
                icon={
                  <UserAvatar
                    avatarUrl={user.avatarUrl}
                    displayName={user.displayName ?? "?"}
                    size="xs"
                  />
                }
                key={user.pubkey}
                label={user.displayName ?? user.pubkey.slice(0, 8)}
                meta={user.nip05Handle}
                onSelect={() => {
                  onInvitedChange([
                    ...invited,
                    {
                      pubkey: user.pubkey,
                      displayName: user.displayName,
                      avatarUrl: user.avatarUrl,
                    },
                  ]);
                  setQuery("");
                }}
              />
            ))}
          {query.trim() && !isSearching && results.length === 0 ? (
            <div className={cn("px-2 py-3 text-sm text-muted-foreground")}>
              No people found
            </div>
          ) : null}
        </div>
      </PopoverContent>
    </Popover>
  );
}
