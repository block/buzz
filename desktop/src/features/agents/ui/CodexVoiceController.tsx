import { Mic } from "lucide-react";
import * as React from "react";
import { useLocation } from "@tanstack/react-router";

import { CodexVoiceDock } from "@/features/agents/ui/CodexVoiceDock";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { subscribeVoiceRoomCommandRequests } from "@/features/agents/observerRelayStore";
import {
  endVoiceTarget,
  hasVoiceTarget,
  startVoiceTarget,
  useCodexVoiceTargets,
  useSavedCodexVoiceTargets,
  type CodexVoiceTargetInput,
} from "@/features/agents/voiceSessionRegistry";
import {
  type CodexVoiceMode,
  getCodexVoiceLinkRevision,
  getCodexVoiceCapability,
  getCodexVoiceTargetLink,
  subscribeCodexVoiceLinkChanges,
} from "@/shared/api/codexVoice";
import { Button } from "@/shared/ui/button";
import {
  executeVoiceRoomCommand,
  installVoiceRoomCommandBridge,
  updateVoiceRoomCommandContext,
} from "@/features/agents/voiceRoomService";

export function CodexVoiceController() {
  const activeTargets = useCodexVoiceTargets();
  const savedTargets = useSavedCodexVoiceTargets();
  const agentsQuery = useManagedAgentsQuery();
  const voiceLinkRevision = React.useSyncExternalStore(
    subscribeCodexVoiceLinkChanges,
    getCodexVoiceLinkRevision,
    getCodexVoiceLinkRevision,
  );
  const [availableTargets, setAvailableTargets] = React.useState<
    CodexVoiceTargetInput[]
  >([]);
  const location = useLocation();
  const isVoiceMode = location.pathname === "/voice";
  React.useEffect(() => installVoiceRoomCommandBridge(), []);
  React.useEffect(
    () =>
      subscribeVoiceRoomCommandRequests((_agentPubkey, request) => {
        executeVoiceRoomCommand(request.command);
      }),
    [],
  );
  React.useEffect(() => {
    void voiceLinkRevision;
    let cancelled = false;
    const agents = agentsQuery.data ?? [];
    void Promise.all(
      agents
        .filter(
          (agent) => agent.name.trim().toLowerCase() !== "github buzz sync",
        )
        .map(async (agent): Promise<CodexVoiceTargetInput | null> => {
          const saved = savedTargets.find(
            (target) =>
              target.agentPubkey.toLowerCase() === agent.pubkey.toLowerCase(),
          );
          const [capability, link] = await Promise.all([
            getCodexVoiceCapability(agent.pubkey, agent.relayUrl),
            getCodexVoiceTargetLink(agent.pubkey),
          ]);
          const resolvedLink =
            link ??
            (saved
              ? { channelId: saved.channelId, threadId: saved.threadId }
              : null);
          const mode = capability.mode ?? saved?.mode ?? null;
          if (!capability.supported || !mode || !resolvedLink) return null;
          return {
            agentName: agent.name,
            agentPubkey: agent.pubkey,
            channelId: resolvedLink.channelId,
            mode,
            relayUrl: agent.relayUrl,
            threadId: resolvedLink.threadId,
            voice: saved?.voice,
          } satisfies CodexVoiceTargetInput;
        }),
    ).then((targets) => {
      if (!cancelled) {
        setAvailableTargets(
          targets.filter(
            (target): target is CodexVoiceTargetInput => target !== null,
          ),
        );
      }
    });
    return () => {
      cancelled = true;
    };
  }, [agentsQuery.data, savedTargets, voiceLinkRevision]);
  React.useEffect(() => {
    updateVoiceRoomCommandContext({ activeTargets, availableTargets });
  }, [activeTargets, availableTargets]);
  return (
    <section
      aria-label="Active voice conversations"
      className={
        isVoiceMode
          ? "hidden"
          : "pointer-events-none fixed bottom-3 right-3 z-45 flex max-h-[calc(100dvh-4.5rem)] w-[min(23rem,calc(100vw-1.5rem))] flex-col-reverse gap-2 overflow-y-auto empty:hidden"
      }
    >
      {activeTargets.map((target) => (
        <CodexVoiceDock
          key={`${target.threadId}:${target.voice}`}
          onEnded={endVoiceTarget}
          target={target}
        />
      ))}
    </section>
  );
}

type CodexVoiceLauncherProps = {
  agentName: string;
  agentPubkey: string;
  channelId: string | null;
  isWorking: boolean;
  relayUrl: string;
  threadId: string | null;
};

export function CodexVoiceLauncher({
  agentName,
  agentPubkey,
  channelId,
  isWorking,
  relayUrl,
  threadId,
}: CodexVoiceLauncherProps) {
  const activeTargets = useCodexVoiceTargets();
  const [supported, setSupported] = React.useState(false);
  const [model, setModel] = React.useState("gpt-live-1-codex");
  const [mode, setMode] = React.useState<CodexVoiceMode | null>(null);

  React.useEffect(() => {
    if (!threadId || !relayUrl) {
      setSupported(false);
      setMode(null);
      return;
    }
    let cancelled = false;
    void getCodexVoiceCapability(agentPubkey, relayUrl)
      .then((capability) => {
        if (cancelled) return;
        setSupported(capability.supported);
        setMode(capability.mode);
        if (capability.model) setModel(capability.model);
      })
      .catch(() => {
        if (!cancelled) {
          setSupported(false);
          setMode(null);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [agentPubkey, relayUrl, threadId]);

  if (!supported || !threadId || !channelId || !mode) return null;

  const active = hasVoiceTarget(activeTargets, threadId);
  const disabled = isWorking || active;

  return (
    <div className="border-t border-border/70 bg-background/92 px-3 pb-[max(0.75rem,env(safe-area-inset-bottom))] pt-3 backdrop-blur-xl">
      <Button
        className="w-full justify-start gap-2 rounded-xl"
        disabled={disabled}
        onClick={() =>
          startVoiceTarget({
            agentName,
            agentPubkey,
            channelId,
            mode,
            relayUrl,
            threadId,
          })
        }
        type="button"
        variant="outline"
      >
        <span className="grid h-7 w-7 place-items-center rounded-full bg-primary text-primary-foreground">
          <Mic className="h-3.5 w-3.5" />
        </span>
        <span className="min-w-0 text-left">
          <span className="block truncate text-sm font-medium">
            {active ? `${agentName} voice is active` : `Talk to ${agentName}`}
          </span>
          <span className="block text-xs font-normal text-muted-foreground">
            {active
              ? "Available everywhere in Buzz"
              : isWorking
                ? "Available after this turn"
                : mode === "proxy"
                  ? `${model} · voice proxy`
                  : `${model} · live room`}
          </span>
        </span>
      </Button>
    </div>
  );
}
