import { Radio, RotateCcw, Send, Swords } from "lucide-react";
import * as React from "react";

import { attachManagedAgentToChannel } from "@/features/agents/channelAgents";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { useChannelsQuery } from "@/features/channels/hooks";
import {
  formatArtilleryChannelMessage,
  formatArtilleryLifecycleMessage,
  formatArtilleryStartMessage,
} from "@/features/games/artillery/channelEvent";
import {
  createArtilleryFinishedEvent,
  createArtilleryStartedEvent,
  createArtilleryTurnResolvedEvent,
} from "@/features/games/artillery/durableProtocol";
import { createManagedArtilleryAgent } from "@/features/games/artillery/liveAgentAdapter";
import { liveArtilleryMatchController } from "@/features/games/artillery/liveMatchController";
import { artilleryRefereeHostSession } from "@/features/games/artillery/refereeHostSession";
import { artilleryRefereeLeaseMs } from "@/features/games/artillery/refereeLease";
import { createArtilleryChannelEnvelope } from "@/features/games/artillery/referee";
import { sendChannelMessage } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";

type SetupStatus = "idle" | "attaching" | "publishing" | "error";

export function LiveMatchSetup() {
  const liveMatch = React.useSyncExternalStore(
    liveArtilleryMatchController.subscribe,
    liveArtilleryMatchController.getSnapshot,
    liveArtilleryMatchController.getSnapshot,
  );
  const agentsQuery = useManagedAgentsQuery();
  const channelsQuery = useChannelsQuery();
  const agents = (agentsQuery.data ?? []).filter((agent) =>
    agent.pubkey.trim(),
  );
  const channels = (channelsQuery.data ?? []).filter(
    (channel) => channel.channelType !== "forum" && !channel.archivedAt,
  );
  const [redPubkey, setRedPubkey] = React.useState("");
  const [bluePubkey, setBluePubkey] = React.useState("");
  const [channelId, setChannelId] = React.useState("");
  const [timeoutSeconds, setTimeoutSeconds] = React.useState(5);
  const [setupStatus, setSetupStatus] = React.useState<SetupStatus>("idle");
  const [setupError, setSetupError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!redPubkey && agents[0]) setRedPubkey(agents[0].pubkey);
    if (!bluePubkey && agents[1]) setBluePubkey(agents[1].pubkey);
  }, [agents, bluePubkey, redPubkey]);
  React.useEffect(() => {
    if (!channelId && channels[0]) {
      const preferred =
        channels.find(
          (channel) => channel.name.toLowerCase() === "agent-lab",
        ) ?? channels[0];
      setChannelId(preferred.id);
    }
  }, [channelId, channels]);

  const redAgent = agents.find((agent) => agent.pubkey === redPubkey);
  const blueAgent = agents.find((agent) => agent.pubkey === bluePubkey);
  const selectedChannel = channels.find((channel) => channel.id === channelId);
  const isActive =
    liveMatch.status === "waiting" || liveMatch.status === "running";
  const invalidPair = Boolean(redAgent && blueAgent && redAgent === blueAgent);

  const startLiveMatch = async () => {
    if (!redAgent || !blueAgent || !selectedChannel || invalidPair) return;
    setSetupStatus("attaching");
    setSetupError(null);
    const matchId = `live-${crypto.randomUUID()}`;
    const maxTurns = 8;
    try {
      const [attachedRed, attachedBlue] = await Promise.all([
        attachManagedAgentToChannel(selectedChannel.id, {
          agent: redAgent,
          ensureRunning: true,
        }),
        attachManagedAgentToChannel(selectedChannel.id, {
          agent: blueAgent,
          ensureRunning: true,
        }),
      ]);
      const statusMessage = await sendChannelMessage(
        selectedChannel.id,
        formatArtilleryStartMessage({
          blueName: attachedBlue.agent.name,
          durableEvent: createArtilleryStartedEvent({
            agents: {
              blue: {
                id: attachedBlue.agent.pubkey,
                name: attachedBlue.agent.name,
              },
              red: {
                id: attachedRed.agent.pubkey,
                name: attachedRed.agent.name,
              },
            },
            matchId,
            maxTurns,
            timeoutMs: timeoutSeconds * 1_000,
          }),
          matchId,
          redName: attachedRed.agent.name,
          timeoutSeconds,
        }),
      );
      const responseTimeoutMs = timeoutSeconds * 1_000;
      const refereeOwnerId = crypto.randomUUID();
      await artilleryRefereeHostSession.start({
        channelId: selectedChannel.id,
        leaseMs: artilleryRefereeLeaseMs(),
        matchId,
        onLeaseLost: () => liveArtilleryMatchController.yieldReferee(),
        ownerId: refereeOwnerId,
        rootEventId: statusMessage.eventId,
        term: 1,
      });
      const red = createManagedArtilleryAgent({
        agent: attachedRed.agent,
        channelId: selectedChannel.id,
        responseTimeoutMs,
        side: "red",
        threadRootEventId: statusMessage.eventId,
      });
      const blue = createManagedArtilleryAgent({
        agent: attachedBlue.agent,
        channelId: selectedChannel.id,
        responseTimeoutMs,
        side: "blue",
        threadRootEventId: statusMessage.eventId,
      });
      setSetupStatus("idle");
      void liveArtilleryMatchController
        .start({
          agents: { red, blue },
          channelId: selectedChannel.id,
          id: matchId,
          maxTurns,
          onMatchComplete: async (match) => {
            await sendChannelMessage(
              selectedChannel.id,
              formatArtilleryLifecycleMessage(
                createArtilleryFinishedEvent(match),
              ),
              statusMessage.eventId,
            );
          },
          onTurnResolved: async ({ state, turn }) => {
            await sendChannelMessage(
              selectedChannel.id,
              formatArtilleryLifecycleMessage(
                createArtilleryTurnResolvedEvent(state, turn),
              ),
              statusMessage.eventId,
            );
          },
          statusEventId: statusMessage.eventId,
          timeoutMs: responseTimeoutMs,
        })
        .then((match) => {
          window.localStorage.setItem(
            "buzz-artillery-last-live-match.v1",
            JSON.stringify(createArtilleryChannelEnvelope(match)),
          );
        })
        .catch(() => {})
        .finally(() => {
          void artilleryRefereeHostSession.stop();
        });
    } catch (cause) {
      setSetupStatus("error");
      setSetupError(cause instanceof Error ? cause.message : "Setup failed");
    }
  };

  const publishResult = async () => {
    if (!liveMatch.match || !liveMatch.channelId) return;
    setSetupStatus("publishing");
    setSetupError(null);
    try {
      await sendChannelMessage(
        liveMatch.channelId,
        formatArtilleryChannelMessage(
          createArtilleryChannelEnvelope(liveMatch.match),
        ),
        liveMatch.statusEventId,
      );
      liveArtilleryMatchController.markPublished();
      setSetupStatus("idle");
    } catch (cause) {
      setSetupStatus("error");
      setSetupError(
        cause instanceof Error ? cause.message : "Couldn't publish result",
      );
    }
  };

  const error = setupError ?? liveMatch.error;

  return (
    <section
      className="rounded-2xl border border-sky-500/20 bg-sky-500/5 p-4"
      data-live-match-status={liveMatch.status}
      data-testid="live-match-setup"
    >
      <div className="flex flex-wrap items-end gap-3">
        <div className="mr-auto max-w-md">
          <div className="flex items-center gap-2 text-sm font-semibold text-foreground">
            <Radio className="h-4 w-4 text-sky-500" aria-hidden="true" />
            Two-agent live match
          </div>
          <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
            Both agents receive correlated requests in one channel thread. Keep
            watching here or return later—the match continues across navigation.
          </p>
        </div>
        <AgentSelect
          agents={agents}
          disabled={isActive || setupStatus === "attaching"}
          label="Red agent"
          onChange={setRedPubkey}
          value={redPubkey}
        />
        <AgentSelect
          agents={agents}
          disabled={isActive || setupStatus === "attaching"}
          label="Blue agent"
          onChange={setBluePubkey}
          value={bluePubkey}
        />
        <label className="grid gap-1 text-xs font-medium text-muted-foreground">
          Game channel
          <select
            className="h-9 min-w-40 rounded-lg border border-input bg-background px-3 text-sm text-foreground"
            disabled={isActive || setupStatus === "attaching"}
            onChange={(event) => setChannelId(event.target.value)}
            value={channelId}
          >
            {channels.length === 0 ? (
              <option value="">No playable channels</option>
            ) : null}
            {channels.map((channel) => (
              <option key={channel.id} value={channel.id}>
                #{channel.name}
              </option>
            ))}
          </select>
        </label>
        <label className="grid gap-1 text-xs font-medium text-muted-foreground">
          Turn timer
          <select
            className="h-9 rounded-lg border border-input bg-background px-3 text-sm text-foreground"
            disabled={isActive || setupStatus === "attaching"}
            onChange={(event) => setTimeoutSeconds(Number(event.target.value))}
            value={timeoutSeconds}
          >
            {[5, 15, 30, 45, 60].map((seconds) => (
              <option key={seconds} value={seconds}>
                {seconds}s
              </option>
            ))}
          </select>
        </label>
        <Button
          data-testid="start-live-artillery-match"
          disabled={
            !redAgent ||
            !blueAgent ||
            !selectedChannel ||
            invalidPair ||
            isActive ||
            setupStatus === "attaching"
          }
          onClick={() => void startLiveMatch()}
          type="button"
          variant="secondary"
        >
          <Swords aria-hidden="true" />
          {setupStatus === "attaching"
            ? "Starting agents…"
            : "Start live match"}
        </Button>
        {liveMatch.status === "complete" && liveMatch.match ? (
          <>
            <Button
              data-testid="publish-artillery-result"
              disabled={setupStatus === "publishing" || liveMatch.published}
              onClick={() => void publishResult()}
              type="button"
              variant="outline"
            >
              <Send aria-hidden="true" />
              {liveMatch.published ? "Published" : "Publish result"}
            </Button>
            <Button
              data-testid="return-to-artillery-demo"
              onClick={() => liveArtilleryMatchController.reset()}
              type="button"
              variant="ghost"
            >
              <RotateCcw aria-hidden="true" /> Demo
            </Button>
          </>
        ) : null}
      </div>
      {invalidPair ? (
        <p className="mt-3 text-sm text-destructive" role="alert">
          Choose two different managed agents.
        </p>
      ) : null}
      {liveMatch.waitingFor ? (
        <WaitingForAgent waitingFor={liveMatch.waitingFor} />
      ) : null}
      {error ? (
        <p className="mt-3 text-sm text-destructive" role="alert">
          {error}
        </p>
      ) : null}
    </section>
  );
}

function AgentSelect({
  agents,
  disabled,
  label,
  onChange,
  value,
}: {
  agents: Array<{ name: string; pubkey: string; status: string }>;
  disabled: boolean;
  label: string;
  onChange: (value: string) => void;
  value: string;
}) {
  return (
    <label className="grid gap-1 text-xs font-medium text-muted-foreground">
      {label}
      <select
        className="h-9 min-w-40 rounded-lg border border-input bg-background px-3 text-sm text-foreground"
        disabled={disabled}
        onChange={(event) => onChange(event.target.value)}
        value={value}
      >
        {agents.length === 0 ? (
          <option value="">No managed agents</option>
        ) : null}
        {agents.map((agent) => (
          <option key={agent.pubkey} value={agent.pubkey}>
            {agent.name} · {agent.status}
          </option>
        ))}
      </select>
    </label>
  );
}

function WaitingForAgent({
  waitingFor,
}: {
  waitingFor: {
    agentName: string;
    deadlineAt: number;
    side: "red" | "blue";
    turn: number;
  };
}) {
  const [now, setNow] = React.useState(Date.now());
  React.useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(timer);
  }, []);
  const secondsRemaining = Math.max(
    0,
    Math.ceil((waitingFor.deadlineAt - now) / 1_000),
  );
  return (
    <div
      className="mt-3 flex items-center justify-between gap-3 rounded-xl border border-sky-400/20 bg-background/60 px-3 py-2 text-sm"
      data-agent-side={waitingFor.side}
      data-testid="live-turn-wait"
    >
      <span>
        Turn {waitingFor.turn}: waiting for{" "}
        <strong>{waitingFor.agentName}</strong>
      </span>
      <span className="font-mono text-xs text-muted-foreground">
        fallback in {secondsRemaining}s
      </span>
    </div>
  );
}
