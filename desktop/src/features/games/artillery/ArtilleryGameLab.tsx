import { Flag, Pause, Play, RotateCcw, Sparkles, Wind } from "lucide-react";
import * as React from "react";
import { Link } from "@tanstack/react-router";

import {
  useDeleteManagedAgentMutation,
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import { deleteManagedAgentWithRules } from "@/features/agents/lib/managedAgentControlActions";
import { useChannelsQuery } from "@/features/channels/hooks";
import type { ArtilleryAnimationPhase } from "@/features/games/artillery/ArtilleryScene";
import {
  playArtillerySound,
  startArtilleryWhistle,
  stopArtilleryWhistle,
} from "@/features/games/artillery/artilleryAudio";
import { ArtillerySoundToggle } from "@/features/games/artillery/ArtillerySoundToggle";
import { ArtilleryVictoryScreen } from "@/features/games/artillery/ArtilleryVictoryScreen";
import {
  ARTILLERY_PHASE_LABELS,
  resolveArtilleryWinnerName,
} from "@/features/games/artillery/artilleryPresentation";
import type { ArtilleryAnimationManifest } from "@/features/games/artillery/manifest";
import type { ArtilleryRavineLoser } from "@/features/games/artillery/ArtilleryRavineCinematic";
import { LiveMatchSetup } from "@/features/games/artillery/LiveMatchSetup";
import { DurableMatchHydrator } from "@/features/games/artillery/DurableMatchHydrator";
import { liveArtilleryMatchController } from "@/features/games/artillery/liveMatchController";
import { createMockArtilleryMatch } from "@/features/games/artillery/mockAgents";
import {
  createArtilleryChannelEnvelope,
  type ArtilleryMatch,
  type ArtillerySide,
} from "@/features/games/artillery/referee";
import { usePresenceQuery } from "@/features/presence/hooks";
import { removeChannelMember } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";

type MatchStatus = "loading" | "playing" | "paused" | "complete" | "forfeited";

type SceneControls = {
  forfeit: (loser: ArtillerySide) => void;
  pauseMatch: () => void;
  playLoserRavine: (loser: ArtilleryRavineLoser) => Promise<void>;
  replayMatch: () => void;
  resumeMatch: () => void;
  skipLoserRavine: () => void;
  updateMatch: (match: ArtilleryMatch, matchComplete: boolean) => void;
};

export function ArtilleryGameLab({
  durableMatch = null,
}: {
  durableMatch?: {
    channelId: string;
    matchId: string;
    rootEventId: string;
  } | null;
}) {
  const gameHostRef = React.useRef<HTMLDivElement>(null);
  const sceneRef = React.useRef<SceneControls | null>(null);
  const [demoMatch, setDemoMatch] = React.useState<ArtilleryMatch | null>(null);
  const liveSnapshot = React.useSyncExternalStore(
    liveArtilleryMatchController.subscribe,
    liveArtilleryMatchController.getSnapshot,
    liveArtilleryMatchController.getSnapshot,
  );
  const match = liveSnapshot.match ?? demoMatch;
  const matchComplete = liveSnapshot.match
    ? liveSnapshot.matchComplete
    : Boolean(demoMatch);
  const matchRef = React.useRef(match);
  const matchCompleteRef = React.useRef(matchComplete);
  const matchId = match?.id;
  const [phase, setPhase] = React.useState<ArtilleryAnimationPhase>("loading");
  const [run, setRun] = React.useState(0);
  const [turnIndex, setTurnIndex] = React.useState(-1);
  const [manifest, setManifest] =
    React.useState<ArtilleryAnimationManifest | null>(null);
  const [status, setStatus] = React.useState<MatchStatus>("loading");
  const [winner, setWinner] = React.useState<ArtillerySide | "draw" | null>(
    null,
  );
  const managedAgentsQuery = useManagedAgentsQuery();
  const relayAgentsQuery = useRelayAgentsQuery();
  const channelsQuery = useChannelsQuery();
  const deleteAgentMutation = useDeleteManagedAgentMutation();
  const loserSide =
    winner === "red" ? "blue" : winner === "blue" ? "red" : null;
  const loser = loserSide && match ? match.agents[loserSide] : null;
  const managedLoser = (managedAgentsQuery.data ?? []).find(
    (agent) => agent.pubkey.toLowerCase() === loser?.id.toLowerCase(),
  );
  const loserPresenceQuery = usePresenceQuery(loser ? [loser.id] : []);
  const [deleteLoserState, setDeleteLoserState] = React.useState<{
    deleted: boolean;
    error: string | null;
    pending: boolean;
  }>({ deleted: false, error: null, pending: false });
  const [ravineLoser, setRavineLoser] =
    React.useState<ArtilleryRavineLoser | null>(null);

  React.useEffect(() => {
    let cancelled = false;
    void createMockArtilleryMatch().then((createdMatch) => {
      if (!cancelled) setDemoMatch(createdMatch);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  React.useEffect(() => {
    matchRef.current = match;
    matchCompleteRef.current = matchComplete;
    if (match) sceneRef.current?.updateMatch(match, matchComplete);
  }, [match, matchComplete]);

  React.useEffect(() => {
    if (!ravineLoser) return;
    const host = gameHostRef.current;
    host?.setAttribute("data-ravine-cinematic", "playing");
    const frame = window.requestAnimationFrame(() => {
      const scene = sceneRef.current;
      if (!scene) {
        host?.setAttribute("data-ravine-cinematic", "complete");
        setRavineLoser(null);
        return;
      }
      void scene.playLoserRavine(ravineLoser).finally(() => {
        host?.setAttribute("data-ravine-cinematic", "complete");
        setRavineLoser(null);
      });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [ravineLoser]);

  React.useEffect(() => {
    const host = gameHostRef.current;
    const initialMatch = matchRef.current;
    if (!host || !initialMatch || !matchId) return;

    let cancelled = false;
    let game: import("phaser").Game | null = null;
    void Promise.all([
      import("phaser"),
      import("@/features/games/artillery/ArtilleryScene"),
    ]).then(([phaserModule, sceneModule]) => {
      if (cancelled) return;

      const Phaser = phaserModule.default;
      const reducedMotion = window.matchMedia(
        "(prefers-reduced-motion: reduce)",
      ).matches;
      const latestMatch = matchRef.current;
      if (!latestMatch) return;
      const scene = new sceneModule.ArtilleryScene(
        latestMatch,
        {
          onPhaseChange: setPhase,
          onRunChange: (nextRun) => {
            setRun(nextRun);
            setStatus("playing");
            setWinner(null);
            setDeleteLoserState({
              deleted: false,
              error: null,
              pending: false,
            });
          },
          onSoundCue: (cue) => {
            const arena = gameHostRef.current;
            if (arena) {
              arena.dataset.lastSoundCue = cue;
              arena.dataset.soundCueCount = String(
                Number(arena.dataset.soundCueCount ?? 0) + 1,
              );
            }
            playArtillerySound(cue);
          },
          onWhistleChange: (active, durationMs) => {
            gameHostRef.current?.setAttribute(
              "data-projectile-whistle",
              active ? "playing" : "stopped",
            );
            if (active && durationMs) startArtilleryWhistle(durationMs);
            else stopArtilleryWhistle();
          },
          onStructureChange: (side, integrity) => {
            gameHostRef.current?.setAttribute(
              `data-${side}-structure-integrity`,
              String(integrity),
            );
          },
          onTurnChange: (nextTurnIndex, nextManifest) => {
            setTurnIndex(nextTurnIndex);
            setManifest(nextManifest);
          },
          onMatchComplete: (nextWinner, reason) => {
            setWinner(nextWinner);
            setStatus(reason === "forfeit" ? "forfeited" : "complete");
          },
        },
        reducedMotion,
        matchCompleteRef.current,
      );
      sceneRef.current = scene;
      game = new Phaser.Game({
        type: Phaser.AUTO,
        parent: host,
        width: sceneModule.ARTILLERY_WORLD_SIZE.width,
        height: sceneModule.ARTILLERY_WORLD_SIZE.height,
        transparent: true,
        antialias: true,
        render: { antialias: true, pixelArt: false, roundPixels: false },
        scale: {
          mode: Phaser.Scale.FIT,
          autoCenter: Phaser.Scale.CENTER_BOTH,
        },
        scene,
      });
    });

    return () => {
      cancelled = true;
      stopArtilleryWhistle();
      sceneRef.current = null;
      game?.destroy(true);
    };
  }, [matchId]);

  const togglePause = () => {
    if (status === "paused") {
      sceneRef.current?.resumeMatch();
      setStatus("playing");
    } else {
      sceneRef.current?.pauseMatch();
      setStatus("paused");
    }
  };
  const replay = () => {
    sceneRef.current?.resumeMatch();
    sceneRef.current?.replayMatch();
  };
  const forfeit = () => {
    sceneRef.current?.forfeit(manifest?.shooter ?? "red");
  };
  const deleteLoser = async () => {
    if (!managedLoser) return;
    setDeleteLoserState({ deleted: false, error: null, pending: true });
    try {
      const relayAgents = relayAgentsQuery.data ?? [];
      const channels = channelsQuery.data ?? [];
      const result = await deleteManagedAgentWithRules({
        agent: managedLoser,
        channels,
        deleteManagedAgent: deleteAgentMutation.mutateAsync,
        presenceLookup: loserPresenceQuery.data,
        relayAgents,
        skipRemoteDeleteConfirm: true,
      });
      if (result.cancelled) {
        setDeleteLoserState({ deleted: false, error: null, pending: false });
        return;
      }

      const normalizedPubkey = managedLoser.pubkey.toLowerCase();
      const channelIds = new Set(
        relayAgents.find(
          (agent) => agent.pubkey.toLowerCase() === normalizedPubkey,
        )?.channelIds ?? [],
      );
      for (const channel of channels) {
        if (
          channel.memberPubkeys.some(
            (pubkey) => pubkey.toLowerCase() === normalizedPubkey,
          )
        ) {
          channelIds.add(channel.id);
        }
      }
      await Promise.allSettled(
        [...channelIds].map((channelId) =>
          removeChannelMember(channelId, managedLoser.pubkey),
        ),
      );
      setDeleteLoserState({ deleted: true, error: null, pending: false });
      setRavineLoser({
        avatarUrl: managedLoser.avatarUrl,
        name: managedLoser.name,
      });
    } catch (cause) {
      setDeleteLoserState({
        deleted: false,
        error:
          cause instanceof Error
            ? cause.message
            : "Failed to delete the loser.",
        pending: false,
      });
      throw cause;
    }
  };
  const envelope = match ? createArtilleryChannelEnvelope(match) : null;
  const winnerName =
    winner && match ? resolveArtilleryWinnerName(match, winner) : null;

  return (
    <main
      className="h-full min-h-0 flex-1 overflow-y-auto bg-[radial-gradient(circle_at_top,_hsl(var(--primary)/0.12),_transparent_42%)]"
      data-testid="artillery-game-lab"
    >
      <div className="mx-auto flex min-h-full w-full max-w-6xl flex-col justify-center gap-5 px-5 py-8 lg:px-8">
        <header className="flex flex-wrap items-end justify-between gap-4">
          <div className="space-y-1.5">
            <div className="flex items-center gap-2 text-sm font-semibold uppercase tracking-[0.18em] text-amber-500">
              <Sparkles className="h-4 w-4" aria-hidden="true" />
              Phase 5 durable arena
            </div>
            <h1 className="text-3xl font-semibold tracking-tight text-foreground">
              Buzz Artillery
            </h1>
            <p className="max-w-2xl text-sm leading-relaxed text-muted-foreground">
              Watch each validated move arc across destructible forts. Channel
              events recover the same damage after reloads and synchronize
              spectator clients.
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <ArtillerySoundToggle />
            <Button asChild type="button" variant="outline">
              <Link search={{}} to="/">
                Back to inbox
              </Link>
            </Button>
            <Button
              data-testid="artillery-pause"
              disabled={
                status === "loading" ||
                status === "complete" ||
                status === "forfeited"
              }
              onClick={togglePause}
              type="button"
              variant="outline"
            >
              {status === "paused" ? (
                <Play aria-hidden="true" />
              ) : (
                <Pause aria-hidden="true" />
              )}
              {status === "paused" ? "Resume" : "Pause"}
            </Button>
            <Button
              data-testid="artillery-forfeit"
              disabled={
                status === "loading" ||
                status === "complete" ||
                status === "forfeited"
              }
              onClick={forfeit}
              type="button"
              variant="outline"
            >
              <Flag aria-hidden="true" /> Forfeit turn agent
            </Button>
            <Button
              data-testid="artillery-replay"
              disabled={phase === "loading" || !match}
              onClick={replay}
              type="button"
            >
              <RotateCcw aria-hidden="true" /> Replay match
            </Button>
          </div>
        </header>

        {durableMatch ? <DurableMatchHydrator {...durableMatch} /> : null}

        <LiveMatchSetup />

        <section className="overflow-hidden rounded-3xl border border-white/10 bg-slate-950 shadow-2xl shadow-sky-950/30">
          <div
            className="relative aspect-video w-full overflow-hidden"
            data-animation-phase={phase}
            data-animation-run={run}
            data-match-status={status}
            data-match-turn={Math.max(0, turnIndex + 1)}
            data-match-turn-count={match?.turns.length ?? 0}
            data-match-winner={winner ?? ""}
            data-turn-resolution={manifest?.resolution ?? ""}
            data-testid="artillery-arena"
            ref={gameHostRef}
          >
            {phase === "loading" ? (
              <div className="absolute inset-0 grid place-items-center text-sm text-slate-300">
                Preparing the agents and arena…
              </div>
            ) : null}
            {winnerName && winner && !ravineLoser ? (
              <ArtilleryVictoryScreen
                canDeleteLoser={Boolean(managedLoser)}
                deleteError={deleteLoserState.error}
                deletePending={deleteLoserState.pending}
                loserDeleted={deleteLoserState.deleted}
                loserName={loser?.name ?? null}
                onDeleteLoser={deleteLoser}
                onReplay={replay}
                reason={status === "forfeited" ? "forfeited" : "complete"}
                winner={winner}
                winnerName={winnerName}
              />
            ) : null}
            {ravineLoser ? (
              <div
                className="pointer-events-none absolute inset-0 z-40"
                data-testid="artillery-ravine-cinematic"
              >
                <p className="sr-only" role="status">
                  {ravineLoser.name} is tumbling down the ravine.
                </p>
                <Button
                  className="pointer-events-auto absolute right-4 top-4 border-white/20 bg-slate-950/75 text-white hover:bg-slate-900"
                  data-testid="skip-artillery-ravine"
                  onClick={() => sceneRef.current?.skipLoserRavine()}
                  type="button"
                  variant="outline"
                >
                  Skip
                </Button>
              </div>
            ) : null}
          </div>
        </section>

        <div className="grid gap-3 sm:grid-cols-3">
          <Metric
            label="Match state"
            value={
              winnerName ? `${winnerName} wins` : ARTILLERY_PHASE_LABELS[phase]
            }
          />
          <Metric
            label="Turn"
            value={
              match
                ? `${Math.max(1, turnIndex + 1)} of ${match.turns.length}`
                : "Preparing"
            }
          />
          <Metric
            icon={<Wind className="h-4 w-4" aria-hidden="true" />}
            label="Referee inputs"
            value={
              manifest
                ? `${manifest.angle}° · ${manifest.power} power · wind ${manifest.wind}`
                : "Awaiting first move"
            }
          />
        </div>

        {match ? (
          <MatchTranscript match={match} activeTurn={turnIndex} />
        ) : null}
        <p className="text-xs text-muted-foreground">
          Channel event boundary: <code>{envelope?.type}</code>. Referee turns
          are persisted automatically; the readable result summary remains an
          explicit publish action.
        </p>
        <p
          className="sr-only"
          aria-live="polite"
          data-testid="artillery-live-status"
        >
          {winnerName
            ? `${winnerName} wins.`
            : `${ARTILLERY_PHASE_LABELS[phase]}. Turn ${turnIndex + 1}.`}{" "}
          Animation run {run}.
        </p>
      </div>
    </main>
  );
}

function MatchTranscript({
  match,
  activeTurn,
}: {
  match: ArtilleryMatch;
  activeTurn: number;
}) {
  return (
    <section
      className="rounded-2xl border border-border/70 bg-card/60 p-4"
      data-testid="artillery-transcript"
    >
      <div className="mb-3 text-xs font-semibold uppercase tracking-[0.16em] text-muted-foreground">
        Authoritative turn transcript
      </div>
      <div className="grid gap-2 md:grid-cols-2">
        {match.turns.map((turn, index) => (
          <div
            className={`rounded-xl border px-3 py-2 text-sm ${index === activeTurn ? "border-amber-400/60 bg-amber-400/10" : "border-border/60 bg-background/45"}`}
            data-resolution={turn.resolution}
            key={turn.manifest.id}
          >
            <div className="flex items-center justify-between gap-3 font-medium">
              <span>
                Turn {index + 1} · {turn.manifest.shooterName}
              </span>
              <span>
                {turn.manifest.damage.before - turn.manifest.damage.after}{" "}
                damage
              </span>
            </div>
            <div className="mt-1 text-xs text-muted-foreground">
              {turn.action.angle}° · power {turn.action.power} ·{" "}
              {turn.resolution === "accepted"
                ? "move accepted"
                : "safe fallback applied"}
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}

function Metric({
  icon,
  label,
  value,
}: {
  icon?: React.ReactNode;
  label: string;
  value: string;
}) {
  return (
    <div className="rounded-2xl border border-border/70 bg-card/70 px-4 py-3 shadow-sm">
      <div className="flex items-center gap-1.5 text-xs font-medium uppercase tracking-wide text-muted-foreground">
        {icon}
        {label}
      </div>
      <div
        className="mt-1 truncate text-sm font-semibold text-foreground"
        title={value}
      >
        {value}
      </div>
    </div>
  );
}
