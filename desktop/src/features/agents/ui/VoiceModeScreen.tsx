import {
  Activity,
  Bot,
  Captions,
  Headphones,
  LoaderCircle,
  Mic,
  MicOff,
  PhoneOff,
  Play,
  Radio,
  Settings2,
  UsersRound,
  Volume2,
  VolumeX,
} from "lucide-react";
import * as React from "react";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import {
  saveVoiceTargetPreference,
  useCodexVoiceSessionStates,
  useCodexVoiceTargets,
  useSavedCodexVoiceTargets,
  useVoiceRoomOutputMuted,
  useVoiceRoomTranscript,
  VOICE_ROOM_PALETTE,
  type CodexVoiceTarget,
  type CodexVoiceTargetInput,
  type VoiceRoomTranscriptEntry,
} from "@/features/agents/voiceSessionRegistry";
import { ManagedAgentSessionPanel } from "@/features/agents/ui/ManagedAgentSessionPanel";
import {
  executeVoiceRoomCommand,
  updateVoiceRoomCommandContext,
} from "@/features/agents/voiceRoomService";
import {
  getCodexVoiceLinkRevision,
  getCodexVoiceCapability,
  getCodexVoiceTargetLink,
  subscribeCodexVoiceLinkChanges,
} from "@/shared/api/codexVoice";
import type { ManagedAgent } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { PageHeader, SubsectionLabel } from "@/shared/ui/PageHeader";

type VoiceAvailability =
  | { kind: "loading" }
  | { kind: "ready"; target: CodexVoiceTargetInput }
  | { kind: "unavailable"; reason: string };

export function VoiceModeScreen() {
  const agentsQuery = useManagedAgentsQuery();
  const activeTargets = useCodexVoiceTargets();
  const savedTargets = useSavedCodexVoiceTargets();
  const sessionStates = useCodexVoiceSessionStates();
  const roomTranscript = useVoiceRoomTranscript();
  const roomOutputMuted = useVoiceRoomOutputMuted();
  const voiceLinkRevision = React.useSyncExternalStore(
    subscribeCodexVoiceLinkChanges,
    getCodexVoiceLinkRevision,
    getCodexVoiceLinkRevision,
  );
  const [availability, setAvailability] = React.useState<
    Record<string, VoiceAvailability>
  >({});
  const [selectedPubkey, setSelectedPubkey] = React.useState<string | null>(
    null,
  );
  const agents = React.useMemo(
    () => agentsQuery.data ?? [],
    [agentsQuery.data],
  );
  const rosterAgents = React.useMemo(
    () => buildVoiceRoster(agents, activeTargets, savedTargets, availability),
    [activeTargets, agents, availability, savedTargets],
  );

  React.useEffect(() => {
    void voiceLinkRevision;
    let cancelled = false;
    setAvailability(
      Object.fromEntries(
        agents.map((agent) => [
          agent.pubkey.toLowerCase(),
          { kind: "loading" } satisfies VoiceAvailability,
        ]),
      ),
    );
    for (const agent of agents) {
      const key = agent.pubkey.toLowerCase();
      const saved = savedTargets.find(
        (target) => target.agentPubkey.toLowerCase() === key,
      );
      void Promise.all([
        getCodexVoiceCapability(agent.pubkey, agent.relayUrl),
        getCodexVoiceTargetLink(agent.pubkey),
      ])
        .then(([capability, link]) => {
          if (cancelled) return;
          const resolvedLink =
            link ??
            (saved
              ? { channelId: saved.channelId, threadId: saved.threadId }
              : null);
          const mode = capability.mode ?? saved?.mode ?? null;
          const result: VoiceAvailability =
            capability.supported && mode && resolvedLink
              ? {
                  kind: "ready",
                  target: {
                    agentName: agent.name,
                    agentPubkey: agent.pubkey,
                    channelId: resolvedLink.channelId,
                    mode,
                    relayUrl: agent.relayUrl,
                    threadId: resolvedLink.threadId,
                    voice: saved?.voice,
                  },
                }
              : {
                  kind: "unavailable",
                  reason:
                    capability.reason ??
                    (capability.supported
                      ? "Start one task with this agent to establish voice context."
                      : "Voice is unavailable for this agent."),
                };
          setAvailability((current) => ({ ...current, [key]: result }));
        })
        .catch((error) => {
          if (cancelled) return;
          setAvailability((current) => ({
            ...current,
            [key]: {
              kind: "unavailable",
              reason:
                error instanceof Error
                  ? error.message
                  : "Voice capability could not be checked.",
            },
          }));
        });
    }
    return () => {
      cancelled = true;
    };
  }, [agents, savedTargets, voiceLinkRevision]);

  React.useEffect(() => {
    if (
      selectedPubkey &&
      rosterAgents.some(
        (agent) => agent.pubkey.toLowerCase() === selectedPubkey.toLowerCase(),
      )
    ) {
      return;
    }
    setSelectedPubkey(
      activeTargets[0]?.agentPubkey ?? rosterAgents[0]?.pubkey ?? null,
    );
  }, [activeTargets, rosterAgents, selectedPubkey]);

  const selectedAgent = rosterAgents.find(
    (agent) => agent.pubkey.toLowerCase() === selectedPubkey?.toLowerCase(),
  );
  const selectedTarget = activeTargets.find(
    (target) =>
      target.agentPubkey.toLowerCase() === selectedPubkey?.toLowerCase(),
  );
  const selectedState = selectedTarget
    ? sessionStates[selectedTarget.threadId]
    : null;
  const selectedCapability = selectedAgent
    ? availability[selectedAgent.pubkey.toLowerCase()]
    : null;
  const selectedSaved = selectedAgent
    ? savedTargets.find(
        (target) =>
          target.agentPubkey.toLowerCase() ===
          selectedAgent.pubkey.toLowerCase(),
      )
    : null;
  const selectedVoice =
    selectedTarget?.voice ??
    (selectedCapability?.kind === "ready"
      ? selectedCapability.target.voice
      : null) ??
    selectedSaved?.voice ??
    VOICE_ROOM_PALETTE[0];
  const roomMuted =
    activeTargets.length > 0 && activeTargets.every((target) => target.muted);
  const activeSpeaker =
    activeTargets.find(
      (target) => sessionStates[target.threadId]?.phase === "listening",
    ) ?? activeTargets[0];
  const activeSpeakerState = activeSpeaker
    ? sessionStates[activeSpeaker.threadId]
    : null;

  React.useEffect(() => {
    updateVoiceRoomCommandContext({
      activeTargets,
      availableTargets: Object.values(availability).flatMap((entry) =>
        entry.kind === "ready" ? [entry.target] : [],
      ),
    });
  }, [activeTargets, availability]);

  return (
    <main className="h-full overflow-y-auto bg-background">
      <div className="mx-auto flex min-h-full w-full max-w-[96rem] flex-col gap-6 px-5 py-6 lg:px-8">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <PageHeader
            description="A persistent room for your agent team, their work, and every voice."
            title="Voice"
          />
          <RoomControls
            activeTargets={activeTargets}
            outputMuted={roomOutputMuted}
            roomMuted={roomMuted}
          />
        </div>

        <section className="grid min-h-[40rem] gap-5 xl:grid-cols-[17rem_minmax(24rem,1fr)_20rem]">
          <ParticipantRail
            activeTargets={activeTargets}
            agents={rosterAgents}
            availability={availability}
            onSelect={setSelectedPubkey}
            selectedPubkey={selectedPubkey}
            sessionStates={sessionStates}
          />

          <div className="flex min-w-0 flex-col gap-5">
            <VoiceStage
              activeSpeaker={activeSpeaker}
              activeSpeakerState={activeSpeakerState}
              agentCount={activeTargets.length}
            />
            <VoiceConversation entries={roomTranscript} />
          </div>

          <AgentInspector
            agent={selectedAgent}
            availability={selectedCapability}
            selectedVoice={selectedVoice}
            sessionState={selectedState}
            target={selectedTarget}
          />
        </section>
      </div>
    </main>
  );
}

function ParticipantRail({
  activeTargets,
  agents,
  availability,
  onSelect,
  selectedPubkey,
  sessionStates,
}: {
  activeTargets: readonly CodexVoiceTarget[];
  agents: ManagedAgent[];
  availability: Record<string, VoiceAvailability>;
  onSelect: (pubkey: string) => void;
  selectedPubkey: string | null;
  sessionStates: ReturnType<typeof useCodexVoiceSessionStates>;
}) {
  return (
    <aside className="min-w-0 rounded-2xl border border-border/70 bg-card/70 p-2 shadow-sm">
      <div className="flex items-center justify-between px-2 pb-2 pt-1">
        <SubsectionLabel>Team</SubsectionLabel>
        <span className="text-xs tabular-nums text-muted-foreground">
          {activeTargets.length} active
        </span>
      </div>
      <div className="max-h-[calc(100dvh-13rem)] space-y-1 overflow-y-auto">
        {agents.map((agent) => {
          const key = agent.pubkey.toLowerCase();
          const active = activeTargets.find(
            (target) => target.agentPubkey.toLowerCase() === key,
          );
          const state = active ? sessionStates[active.threadId] : null;
          const capability = availability[key];
          const selected = selectedPubkey?.toLowerCase() === key;
          return (
            <button
              className={cn(
                "group flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left transition-colors",
                selected ? "bg-primary/10" : "hover:bg-muted/70",
              )}
              key={agent.pubkey}
              onClick={() => onSelect(agent.pubkey)}
              type="button"
            >
              <span
                className={cn(
                  "relative grid h-9 w-9 shrink-0 place-items-center rounded-full bg-muted text-muted-foreground",
                  active && "bg-primary text-primary-foreground",
                )}
              >
                <Bot className="h-4 w-4" />
                <span
                  className={cn(
                    "absolute bottom-0 right-0 h-2.5 w-2.5 rounded-full border-2 border-card bg-muted-foreground/50",
                    active && state?.phase !== "error" && "bg-emerald-500",
                    state?.phase === "error" && "bg-destructive",
                  )}
                />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-medium">
                  {agent.name}
                </span>
                <span className="block truncate text-xs text-muted-foreground">
                  {voiceStatus(active, state, capability)}
                </span>
              </span>
              {active ? <Headphones className="h-4 w-4 text-primary" /> : null}
            </button>
          );
        })}
      </div>
    </aside>
  );
}

function VoiceStage({
  activeSpeaker,
  activeSpeakerState,
  agentCount,
}: {
  activeSpeaker: CodexVoiceTarget | undefined;
  activeSpeakerState:
    | ReturnType<typeof useCodexVoiceSessionStates>[string]
    | null;
  agentCount: number;
}) {
  const listening = activeSpeakerState?.phase === "listening";
  return (
    <section className="relative flex min-h-[25rem] flex-col items-center justify-center overflow-hidden rounded-2xl border border-border/70 bg-card px-8 py-10 text-center shadow-sm">
      <div className="pointer-events-none absolute inset-x-16 top-0 h-40 rounded-full bg-primary/[0.06] blur-3xl" />
      <div className="relative grid h-40 w-40 place-items-center">
        <span
          className={cn(
            "absolute inset-0 rounded-full border border-primary/15",
            listening &&
              "animate-pulse scale-110 border-primary/30 motion-reduce:animate-none",
          )}
        />
        <span
          className={cn(
            "absolute inset-5 rounded-full border border-primary/20",
            listening &&
              "animate-pulse scale-105 motion-reduce:animate-none [animation-delay:180ms]",
          )}
        />
        <span className="grid h-28 w-28 place-items-center rounded-full bg-gradient-to-b from-primary/90 to-primary text-primary-foreground shadow-lg shadow-primary/15">
          <Radio className="h-10 w-10" strokeWidth={1.5} />
        </span>
      </div>
      <div className="mt-7 flex items-center gap-2 rounded-full border border-border/70 bg-background/70 px-3 py-1 text-xs text-muted-foreground">
        <UsersRound className="h-3.5 w-3.5" />
        {agentCount} {agentCount === 1 ? "agent" : "agents"} in the room
      </div>
      <h2 className="mt-4 text-2xl font-semibold tracking-tight">
        {activeSpeaker ? activeSpeaker.agentName : "Room ready"}
      </h2>
      <p className="mt-2 max-w-lg text-sm leading-relaxed text-muted-foreground">
        {activeSpeakerState?.error ??
          (activeSpeaker
            ? `${activeSpeakerState?.muted ? "Muted" : "Listening"} · ${activeSpeaker.voice} voice`
            : "Invite an agent from the team rail to begin. The room follows you throughout Buzz.")}
      </p>
    </section>
  );
}

function VoiceConversation({
  entries,
}: {
  entries: readonly VoiceRoomTranscriptEntry[];
}) {
  const scrollRef = React.useRef<HTMLDivElement>(null);
  const [followingLatest, setFollowingLatest] = React.useState(true);
  const entryCount = entries.length;
  const latestEntryId = entries.at(-1)?.id ?? null;
  React.useEffect(() => {
    if (latestEntryId === null) return;
    const scroller = scrollRef.current;
    if (followingLatest && scroller) scroller.scrollTop = scroller.scrollHeight;
  }, [followingLatest, latestEntryId]);
  const jumpToLatest = React.useCallback(() => {
    const scroller = scrollRef.current;
    if (!scroller) return;
    scroller.scrollTop = scroller.scrollHeight;
    setFollowingLatest(true);
  }, []);
  return (
    <section className="overflow-hidden rounded-2xl border border-border/70 bg-card shadow-sm">
      <div className="flex items-center justify-between border-b border-border/70 px-4 py-3">
        <SubsectionLabel>Conversation</SubsectionLabel>
        <div className="flex items-center gap-3 text-xs text-muted-foreground">
          {!followingLatest ? (
            <button
              className="font-medium text-foreground hover:underline"
              onClick={jumpToLatest}
              type="button"
            >
              Jump to latest
            </button>
          ) : null}
          <span className="flex items-center gap-1.5">
            <Captions className="h-3.5 w-3.5" />
            {entryCount} {entryCount === 1 ? "turn" : "turns"}
          </span>
        </div>
      </div>
      <div
        aria-label="Voice conversation transcript"
        className="h-72 overflow-y-auto overscroll-contain px-4 py-3"
        onScroll={(event) => {
          const scroller = event.currentTarget;
          const distanceFromBottom =
            scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight;
          setFollowingLatest(distanceFromBottom < 48);
        }}
        ref={scrollRef}
        role="log"
      >
        {entries.length ? (
          <div className="space-y-4">
            {entries.map((entry) => (
              <article
                className={cn(
                  "flex",
                  entry.speakerType === "human" && "justify-end",
                )}
                key={entry.id}
              >
                <div className="max-w-[82%]">
                  <div
                    className={cn(
                      "mb-1 flex items-center gap-2 text-xs text-muted-foreground",
                      entry.speakerType === "human" && "justify-end",
                    )}
                  >
                    <span className="font-medium text-foreground">
                      {entry.speakerName}
                    </span>
                    <time dateTime={new Date(entry.timestamp).toISOString()}>
                      {new Date(entry.timestamp).toLocaleTimeString([], {
                        hour: "numeric",
                        minute: "2-digit",
                      })}
                    </time>
                  </div>
                  <p
                    className={cn(
                      "whitespace-pre-wrap break-words rounded-xl bg-muted/60 px-3 py-2 text-sm leading-relaxed",
                      entry.speakerType === "human" &&
                        "bg-primary text-primary-foreground",
                    )}
                  >
                    {entry.text}
                  </p>
                </div>
              </article>
            ))}
          </div>
        ) : (
          <div className="grid h-full place-items-center text-center text-sm text-muted-foreground">
            <div>
              <Captions className="mx-auto mb-2 h-5 w-5" />
              Voice turns from you and every agent will appear here.
            </div>
          </div>
        )}
      </div>
    </section>
  );
}

function AgentInspector({
  agent,
  availability,
  selectedVoice,
  sessionState,
  target,
}: {
  agent: ManagedAgent | undefined;
  availability: VoiceAvailability | null | undefined;
  selectedVoice: string;
  sessionState: ReturnType<typeof useCodexVoiceSessionStates>[string] | null;
  target: CodexVoiceTarget | undefined;
}) {
  if (!agent) {
    return (
      <aside className="rounded-2xl border border-border/70 bg-card p-5 text-sm text-muted-foreground shadow-sm">
        Select an agent to inspect its voice and activity.
      </aside>
    );
  }
  const readyTarget =
    availability?.kind === "ready" ? availability.target : null;
  return (
    <aside className="min-w-0 rounded-2xl border border-border/70 bg-card p-5 shadow-sm">
      <div className="flex items-start gap-3">
        <span className="grid h-11 w-11 shrink-0 place-items-center rounded-xl bg-primary/10 text-primary">
          <Bot className="h-5 w-5" />
        </span>
        <div className="min-w-0 flex-1">
          <h2 className="truncate font-semibold">{agent.name}</h2>
          <p className="truncate text-xs text-muted-foreground">
            {agent.model ?? agent.runtime ?? "Managed agent"}
          </p>
        </div>
      </div>

      <div className="mt-5 grid grid-cols-2 gap-2 text-xs">
        <Metric
          label="Status"
          value={target ? (sessionState?.phase ?? "Starting") : agent.status}
        />
        <Metric
          label="Transport"
          value={target?.mode ?? readyTarget?.mode ?? "Unavailable"}
        />
      </div>

      <div className="mt-5">
        <label
          className="text-xs font-medium"
          htmlFor={`voice-${agent.pubkey}`}
        >
          Voice
        </label>
        <select
          className="mt-2 h-9 w-full rounded-lg border border-input bg-background px-3 text-sm text-foreground"
          disabled={!target && !readyTarget}
          id={`voice-${agent.pubkey}`}
          onChange={(event) => {
            if (target) {
              executeVoiceRoomCommand({
                action: "set-voice",
                threadId: target.threadId,
                voice: event.target.value,
              });
            } else if (readyTarget) {
              saveVoiceTargetPreference({
                ...readyTarget,
                voice: event.target.value,
              });
            }
          }}
          value={selectedVoice}
        >
          {VOICE_ROOM_PALETTE.map((voice) => (
            <option key={voice} value={voice}>
              {voice[0]?.toUpperCase()}
              {voice.slice(1)}
            </option>
          ))}
        </select>
        <p className="mt-2 text-xs leading-relaxed text-muted-foreground">
          {availability?.kind === "unavailable"
            ? availability.reason
            : target
              ? "Changing voice reconnects only this agent."
              : "Saved for the next time this agent joins."}
        </p>
      </div>

      <div className="mt-5 space-y-2">
        {target ? (
          <>
            <Button
              className="w-full justify-start gap-2"
              onClick={() =>
                executeVoiceRoomCommand({
                  action: "set-muted",
                  muted: !(target.muted ?? false),
                  threadId: target.threadId,
                })
              }
              type="button"
              variant="outline"
            >
              {target.muted ? <Mic /> : <MicOff />}
              {target.muted ? "Unmute agent" : "Mute agent"}
            </Button>
            <Button
              className="w-full justify-start gap-2"
              onClick={() =>
                executeVoiceRoomCommand({
                  action: "remove",
                  threadId: target.threadId,
                })
              }
              type="button"
              variant="outline"
            >
              <PhoneOff /> Remove from room
            </Button>
          </>
        ) : (
          <Button
            className="w-full justify-start gap-2"
            disabled={!readyTarget}
            onClick={() =>
              readyTarget &&
              executeVoiceRoomCommand({
                action: "join",
                threadId: readyTarget.threadId,
              })
            }
            type="button"
          >
            {availability?.kind === "loading" ? (
              <LoaderCircle className="animate-spin" />
            ) : (
              <Play />
            )}
            Join voice
          </Button>
        )}
      </div>

      <div className="mt-6 border-t border-border/70 pt-4">
        <div className="mb-3 flex items-center justify-between gap-2">
          <div className="flex items-center gap-2 text-xs font-medium">
            <Activity className="h-3.5 w-3.5" /> Live work
          </div>
          <span className="text-xs capitalize text-muted-foreground">
            {agent.status}
          </span>
        </div>
        <ManagedAgentSessionPanel
          agent={agent}
          autoTail
          className="h-72 border-0 bg-transparent p-0 shadow-none"
          emptyDescription={`When ${agent.name} starts working, thoughts, tools, and outputs will stream here.`}
          panelPadding={false}
          rawLayout="exclusive"
          showHeader={false}
          showRaw={false}
          transcriptContentClassName="px-0"
          transcriptVariant="compactPreview"
        />
      </div>
      <div className="mt-4 flex items-center gap-2 text-xs text-muted-foreground">
        <Settings2 className="h-3.5 w-3.5" />
        {agent.provider ?? "Local"} · {agent.status}
      </div>
    </aside>
  );
}

function RoomControls({
  activeTargets,
  outputMuted,
  roomMuted,
}: {
  activeTargets: readonly CodexVoiceTarget[];
  outputMuted: boolean;
  roomMuted: boolean;
}) {
  if (!activeTargets.length) return null;
  return (
    <div className="flex items-center gap-2">
      <Button
        className="gap-2"
        onClick={() => {
          for (const target of activeTargets) {
            executeVoiceRoomCommand({
              action: "set-muted",
              muted: !roomMuted,
              threadId: target.threadId,
            });
          }
        }}
        size="sm"
        type="button"
        variant="outline"
      >
        {roomMuted ? <Mic /> : <MicOff />}
        {roomMuted ? "Unmute room" : "Mute room"}
      </Button>
      <Button
        className="gap-2"
        onClick={() =>
          executeVoiceRoomCommand({
            action: "set-output-muted",
            muted: !outputMuted,
          })
        }
        size="sm"
        type="button"
        variant="outline"
      >
        {outputMuted ? <Volume2 /> : <VolumeX />}
        {outputMuted ? "Hear agents" : "Mute agents"}
      </Button>
      <Button
        className="gap-2"
        onClick={() => {
          for (const target of activeTargets) {
            executeVoiceRoomCommand({
              action: "remove",
              threadId: target.threadId,
            });
          }
        }}
        size="sm"
        type="button"
        variant="outline"
      >
        <PhoneOff /> End room
      </Button>
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg bg-muted/55 px-3 py-2">
      <span className="block text-muted-foreground">{label}</span>
      <span className="mt-0.5 block truncate font-medium capitalize text-foreground">
        {value}
      </span>
    </div>
  );
}

function voiceStatus(
  active: CodexVoiceTarget | undefined,
  state: ReturnType<typeof useCodexVoiceSessionStates>[string] | null,
  availability: VoiceAvailability | undefined,
) {
  if (active) {
    if (state?.error) return "Connection issue";
    if (active.muted || state?.muted) return "Muted";
    return state?.phase === "listening" ? "Listening" : "Connecting";
  }
  if (availability?.kind === "loading") return "Checking voice";
  if (availability?.kind === "unavailable") return availability.reason;
  return "Ready to join";
}

function buildVoiceRoster(
  agents: ManagedAgent[],
  activeTargets: readonly CodexVoiceTarget[],
  savedTargets: readonly CodexVoiceTarget[],
  availability: Readonly<Record<string, VoiceAvailability>>,
) {
  const activePubkeys = new Set(
    activeTargets.map((target) => target.agentPubkey.toLowerCase()),
  );
  const savedPubkeys = new Set(
    savedTargets.map((target) => target.agentPubkey.toLowerCase()),
  );
  const grouped = new Map<string, ManagedAgent>();
  for (const agent of agents) {
    if (agent.name.trim().toLowerCase() === "github buzz sync") continue;
    const groupKey = agent.personaId
      ? `persona:${agent.personaId}`
      : `name:${agent.name.trim().toLowerCase()}`;
    const current = grouped.get(groupKey);
    if (!current || agentPriority(agent) > agentPriority(current)) {
      grouped.set(groupKey, agent);
    }
  }
  return [...grouped.values()].sort((left, right) => {
    const leftActive = activePubkeys.has(left.pubkey.toLowerCase()) ? 1 : 0;
    const rightActive = activePubkeys.has(right.pubkey.toLowerCase()) ? 1 : 0;
    if (leftActive !== rightActive) return rightActive - leftActive;
    const leftSaved = savedPubkeys.has(left.pubkey.toLowerCase()) ? 1 : 0;
    const rightSaved = savedPubkeys.has(right.pubkey.toLowerCase()) ? 1 : 0;
    if (leftSaved !== rightSaved) return rightSaved - leftSaved;
    return left.name.localeCompare(right.name);
  });

  function agentPriority(agent: ManagedAgent) {
    const key = agent.pubkey.toLowerCase();
    if (activePubkeys.has(key)) return 5;
    if (availability[key]?.kind === "ready") return 4;
    if (savedPubkeys.has(key)) return 3;
    if (agent.status === "running") return 2;
    return agent.status === "deployed" ? 1 : 0;
  }
}
