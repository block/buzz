import { X } from "lucide-react";
import * as React from "react";

import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import type { VoiceRegistryEntry } from "@/features/settings/ui/voiceSettingsLogic";
import { invokeTauri } from "@/shared/api/tauri";
import { cn } from "@/shared/lib/cn";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import {
  AgentVoiceMenu,
  type HuddleAgentVoiceSettings,
} from "./AgentVoiceMenu";

type ParticipantListProps = {
  /** Pubkey hex strings from the Rust huddle state. */
  participants: string[];
  activeSpeakers?: string[];
  /** Current per-participant voice levels, normalized to 0–1. */
  speakerLevels?: Record<string, number>;
  /** Pubkeys of agent participants. */
  agentPubkeys?: string[];
  /** Local, huddle-scoped playback choices for agent participants. */
  agentVoiceSettings?: Record<string, HuddleAgentVoiceSettings>;
  /** Called when the user clicks the remove button on an agent avatar. */
  onRemoveAgent?: (pubkey: string) => void;
  /** Interrupts local TTS playback while this agent is speaking. */
  onInterruptAgentSpeech?: (pubkey: string) => void;
  /** The locally loaded profile is authoritative for the current participant. */
  selfProfile?: {
    avatarUrl: string | null;
    displayName: string | null;
    pubkey: string | null;
  };
  appearance?: "bar" | "room";
  className?: string;
};

const MAX_VISIBLE_PARTICIPANTS = 9;

type ParticipantIdentity = {
  avatarUrl: string | null;
  displayName: string;
  isActive: boolean;
  isAgent: boolean;
  pubkey: string;
  speakerLevel: number;
};

function buildParticipantIdentities({
  agentPubkeys,
  activeSpeakers,
  speakerLevels,
  participants,
  profiles,
  managedAgents,
  relayAgents,
}: {
  agentPubkeys?: string[];
  activeSpeakers?: string[];
  speakerLevels?: Record<string, number>;
  participants: string[];
  profiles: Record<
    string,
    { avatarUrl: string | null; displayName: string | null }
  >;
  managedAgents?: { avatarUrl: string | null; name: string; pubkey: string }[];
  relayAgents?: { name: string; pubkey: string }[];
}): ParticipantIdentity[] {
  const agentSet = new Set(
    (agentPubkeys ?? []).map((pubkey) => pubkey.toLowerCase()),
  );
  const activeSpeakerSet = new Set(
    (activeSpeakers ?? []).map((pubkey) => pubkey.toLowerCase()),
  );
  const normalizedSpeakerLevels = new Map(
    Object.entries(speakerLevels ?? {}).map(([pubkey, level]) => [
      pubkey.toLowerCase(),
      Math.min(1, Math.max(0, level)),
    ]),
  );
  const agentNames = new Map<
    string,
    { avatarUrl: string | null; name: string }
  >();

  for (const agent of relayAgents ?? []) {
    agentNames.set(agent.pubkey.toLowerCase(), {
      avatarUrl: null,
      name: agent.name,
    });
  }
  for (const agent of managedAgents ?? []) {
    agentNames.set(agent.pubkey.toLowerCase(), {
      avatarUrl: agent.avatarUrl,
      name: agent.name,
    });
  }

  return participants.map((pubkey) => {
    const normalizedPubkey = pubkey.toLowerCase();
    const profile = profiles[normalizedPubkey];
    const isAgent = agentSet.has(normalizedPubkey);
    const agent = agentNames.get(normalizedPubkey);
    const displayName =
      profile?.displayName?.trim() ||
      agent?.name?.trim() ||
      `${isAgent ? "Agent" : "Participant"} ${truncatePubkey(pubkey)}`;
    const speakerLevel =
      normalizedSpeakerLevels.get(normalizedPubkey) ??
      (activeSpeakerSet.has(normalizedPubkey) ? 0.55 : 0);

    return {
      avatarUrl: profile?.avatarUrl ?? agent?.avatarUrl ?? null,
      displayName,
      isActive: activeSpeakerSet.has(normalizedPubkey) || speakerLevel > 0.04,
      isAgent,
      pubkey,
      speakerLevel,
    };
  });
}

export function HuddleParticipantsControl({
  participants,
  activeSpeakers,
  speakerLevels,
  agentPubkeys,
  agentVoiceSettings,
  onRemoveAgent,
  onInterruptAgentSpeech,
  selfProfile,
  appearance = "bar",
  className,
}: ParticipantListProps) {
  const { data } = useUsersBatchQuery(participants);
  const profiles = React.useMemo(() => {
    const resolvedProfiles: Record<
      string,
      { avatarUrl: string | null; displayName: string | null }
    > = { ...(data?.profiles ?? {}) };
    const selfPubkey = selfProfile?.pubkey?.toLowerCase();
    if (selfPubkey && selfProfile) {
      const resolvedProfile = resolvedProfiles[selfPubkey];
      resolvedProfiles[selfPubkey] = {
        avatarUrl: selfProfile.avatarUrl ?? resolvedProfile?.avatarUrl ?? null,
        displayName:
          selfProfile.displayName ?? resolvedProfile?.displayName ?? null,
      };
    }
    return resolvedProfiles;
  }, [data?.profiles, selfProfile]);
  const hasAgents = (agentPubkeys?.length ?? 0) > 0;
  const relayAgentsQuery = useRelayAgentsQuery({ enabled: hasAgents });
  const managedAgentsQuery = useManagedAgentsQuery({ enabled: hasAgents });
  const [voiceRegistry, setVoiceRegistry] = React.useState<
    VoiceRegistryEntry[]
  >([]);
  const [resolvedAgentVoiceSettings, setResolvedAgentVoiceSettings] =
    React.useState<Record<string, HuddleAgentVoiceSettings>>(
      agentVoiceSettings ?? {},
    );
  const agentRosterKey = (agentPubkeys ?? []).join(":");

  React.useEffect(() => {
    if (!hasAgents || !agentRosterKey) {
      setResolvedAgentVoiceSettings({});
      return;
    }
    let disposed = false;
    Promise.all([
      invokeTauri<Record<string, HuddleAgentVoiceSettings>>(
        "ensure_huddle_agent_voice_settings",
      ),
      invokeTauri<VoiceRegistryEntry[]>("list_voice_registry"),
    ])
      .then(([settings, registry]) => {
        if (!disposed) {
          setResolvedAgentVoiceSettings(settings);
          setVoiceRegistry(registry);
        }
      })
      .catch((error) => {
        console.error("Failed to load Huddle agent voices:", error);
      });
    return () => {
      disposed = true;
    };
  }, [agentRosterKey, hasAgents]);

  React.useEffect(() => {
    if (agentVoiceSettings) {
      setResolvedAgentVoiceSettings(agentVoiceSettings);
    }
  }, [agentVoiceSettings]);
  const identities = React.useMemo(
    () =>
      buildParticipantIdentities({
        agentPubkeys,
        activeSpeakers,
        speakerLevels,
        participants,
        profiles,
        managedAgents: managedAgentsQuery.data,
        relayAgents: relayAgentsQuery.data,
      }),
    [
      agentPubkeys,
      activeSpeakers,
      speakerLevels,
      managedAgentsQuery.data,
      participants,
      profiles,
      relayAgentsQuery.data,
    ],
  );

  if (participants.length === 0) return null;

  const participantLabel =
    participants.length === 1
      ? "1 participant"
      : `${participants.length} participants`;
  const visibleIdentities = identities.slice(0, MAX_VISIBLE_PARTICIPANTS);
  const hiddenParticipantCount = identities.length - visibleIdentities.length;
  const participantDetails = (
    <PopoverContent
      align="end"
      className="buzz-huddle-drawer buzz-huddle-popover w-72 p-3 text-foreground"
      side={appearance === "room" ? "bottom" : "top"}
      sideOffset={10}
    >
      <div className="mb-3 flex items-center justify-between gap-3">
        <h2 className="text-sm font-medium">Participants</h2>
        <span className="shrink-0 text-xs text-foreground/60">
          {participantLabel}
        </span>
      </div>
      <ul className="flex max-h-64 list-none flex-col gap-1 overflow-y-auto">
        {identities.map((participant) => {
          const { displayName, isActive, isAgent, pubkey } = participant;

          return (
            <li
              className="flex min-w-0 items-center gap-2 rounded-md px-2 py-1.5"
              key={pubkey}
            >
              <UserProfilePopover
                pubkey={pubkey}
                triggerAriaLabel={`Open profile for ${displayName}`}
                triggerElement="span"
              >
                <ParticipantAvatar participant={participant} size="list" />
              </UserProfilePopover>

              <div className="min-w-0 flex-1">
                <div className="truncate text-sm font-medium">
                  {displayName}
                </div>
                <div className="truncate text-xs text-foreground/60">
                  {isActive ? "Speaking" : isAgent ? "Agent" : "In huddle"}
                </div>
              </div>

              {isAgent && onRemoveAgent && (
                <Button
                  aria-label={`Remove ${displayName} from huddle`}
                  className="h-7 w-7 shrink-0 text-foreground/65 hover:bg-destructive/15 hover:text-destructive"
                  onClick={() => void onRemoveAgent(pubkey)}
                  size="icon"
                  type="button"
                  variant="ghost"
                >
                  <X className="h-4 w-4" />
                </Button>
              )}
            </li>
          );
        })}
      </ul>
    </PopoverContent>
  );

  const participantStrip = (
    <div
      className={cn(
        "relative flex max-w-full shrink items-center justify-start overflow-x-auto px-1",
        appearance === "room" ? "gap-2 py-2" : "h-12 gap-1",
        className,
      )}
      data-testid="huddle-participant-strip"
    >
      {visibleIdentities.map((participant) =>
        appearance === "room" ? (
          <div
            className="buzz-huddle-participant-tile relative flex w-28 shrink-0 flex-col items-center justify-center gap-1.5 rounded-xl border border-border/70 bg-muted/45 px-3 py-3"
            data-testid="huddle-participant-tile"
            key={participant.pubkey}
          >
            {participant.isAgent ? (
              <AgentVoiceMenu
                agentPubkey={participant.pubkey}
                displayName={participant.displayName}
                onRemoveAgent={
                  onRemoveAgent
                    ? () => void onRemoveAgent(participant.pubkey)
                    : undefined
                }
                onSettingsChange={(settings) => {
                  setResolvedAgentVoiceSettings((current) => ({
                    ...current,
                    [participant.pubkey]: settings,
                  }));
                }}
                registry={voiceRegistry}
                settings={resolvedAgentVoiceSettings[participant.pubkey]}
              />
            ) : null}
            <ParticipantAvatar participant={participant} size="room" />
            <div
              className="-mb-2 mt-1 flex h-5 w-full shrink-0 items-center justify-center"
              data-testid="huddle-participant-label-slot"
            >
              {participant.isAgent &&
              participant.isActive &&
              onInterruptAgentSpeech ? (
                <Button
                  aria-label={`Stop ${participant.displayName} speaking`}
                  className="h-5 min-h-0 w-full rounded-sm bg-destructive/15 px-0 py-0 text-xs font-medium leading-none text-destructive hover:bg-destructive/20 hover:text-destructive"
                  onClick={() => onInterruptAgentSpeech(participant.pubkey)}
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  Stop
                </Button>
              ) : (
                <span className="w-full truncate text-center text-xs font-medium leading-none text-foreground/80">
                  {participant.displayName}
                </span>
              )}
            </div>
          </div>
        ) : participant.isAgent ? (
          <AgentVoiceMenu
            agentPubkey={participant.pubkey}
            contentAlign="start"
            contentClassName="buzz-huddle-drawer buzz-huddle-popover text-foreground"
            contentSide="top"
            displayName={participant.displayName}
            key={participant.pubkey}
            onRemoveAgent={
              onRemoveAgent
                ? () => void onRemoveAgent(participant.pubkey)
                : undefined
            }
            onSettingsChange={(settings) => {
              setResolvedAgentVoiceSettings((current) => ({
                ...current,
                [participant.pubkey]: settings,
              }));
            }}
            registry={voiceRegistry}
            settings={resolvedAgentVoiceSettings[participant.pubkey]}
            trigger={
              <button
                aria-label={`Voice settings for ${participant.displayName}`}
                className="inline-flex shrink-0 cursor-pointer rounded-full outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-black"
                data-testid="huddle-agent-voice-menu-trigger"
                type="button"
              >
                <ParticipantAvatar participant={participant} size="bar" />
              </button>
            }
          />
        ) : (
          <Tooltip key={participant.pubkey}>
            <TooltipTrigger asChild>
              <span className="inline-flex shrink-0 rounded-full">
                <ParticipantAvatar participant={participant} size="bar" />
              </span>
            </TooltipTrigger>
            <TooltipContent className="buzz-huddle-tooltip" side="top">
              {participant.displayName}
            </TooltipContent>
          </Tooltip>
        ),
      )}
      {hiddenParticipantCount > 0 ? (
        <PopoverTrigger asChild>
          <Button
            aria-label={`Show all huddle participants (${participants.length})`}
            className={cn(
              "relative z-10 shrink-0 px-1 text-2xs font-semibold shadow-none tabular-nums",
              appearance === "room"
                ? "buzz-huddle-participant-tile min-h-[6.375rem] min-w-28 rounded-xl border border-border/70 bg-muted/45 text-foreground/70 hover:bg-muted/65 hover:text-foreground"
                : "h-9 min-w-9 rounded-full border-2 border-black bg-white/15 text-white hover:bg-white/25 hover:text-white",
            )}
            type="button"
            variant="ghost"
          >
            +{hiddenParticipantCount}
          </Button>
        </PopoverTrigger>
      ) : null}
    </div>
  );

  if (hiddenParticipantCount === 0) return participantStrip;

  return (
    <Popover>
      {participantStrip}
      {participantDetails}
    </Popover>
  );
}

function ParticipantAvatar({
  participant,
  size,
}: {
  participant: ParticipantIdentity;
  size: "bar" | "list" | "room";
}) {
  const sizeClass =
    size === "room" ? "h-14 w-14" : size === "bar" ? "h-9 w-9" : "h-8 w-8";
  const speakerLevel = Math.min(1, Math.max(0, participant.speakerLevel));
  const speakerStyle = {
    "--buzz-huddle-speaker-opacity":
      speakerLevel > 0.04 ? (0.45 + speakerLevel * 0.55).toFixed(3) : "0",
    "--buzz-huddle-speaker-scale": (1.02 + speakerLevel * 0.14).toFixed(3),
  } as React.CSSProperties;

  return (
    <span
      className={cn(
        "buzz-huddle-speaking-avatar relative z-0 inline-flex shrink-0 rounded-full",
        sizeClass,
      )}
      data-testid="huddle-participant-avatar"
      style={speakerStyle}
    >
      <ProfileAvatar
        avatarUrl={participant.avatarUrl}
        label={participant.displayName}
        className="h-full w-full rounded-full border-2 border-black text-2xs"
      />
    </span>
  );
}
