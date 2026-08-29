import * as React from "react";
import { CircleAlert, Mic, MicOff, VideoOff } from "lucide-react";

import type { AgentMediaSession } from "@/features/agents/lib/agentMediaSession";
import { useAgentMediaRoom } from "@/features/agents/lib/useAgentMediaRoom";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";

type AgentMediaSurfaceProps = {
  agentLabel: string;
  className?: string;
  session: AgentMediaSession;
};

/** Attach the remote video track to a `<video>` for as long as both exist. */
function AgentVideoElement({
  attachVideo,
  label,
}: {
  attachVideo: (element: HTMLVideoElement) => () => void;
  label: string;
}) {
  const ref = React.useRef<HTMLVideoElement | null>(null);

  React.useEffect(() => {
    const element = ref.current;
    if (!element) return;
    return attachVideo(element);
  }, [attachVideo]);

  return (
    // Live WebRTC stream: a static track file cannot caption it. The agent's
    // speech is transcribed into the channel as signed events instead, which is
    // the durable accessible record.
    // biome-ignore lint/a11y/useMediaCaption: live stream, see above
    <video
      aria-label={`${label} video`}
      autoPlay
      className="h-full w-full object-cover"
      data-testid="agent-media-video"
      // Not muted: the agent's audio rides its own track in the same room, and
      // hearing it is the point. Lip-sync holds because both come from one
      // transport with one clock.
      playsInline
      ref={ref}
    />
  );
}

/**
 * The live video surface for an agent media session.
 *
 * Renders inside the agent's session panel, above its activity — the face and
 * its work in one place, which is where the conversation already is. The tile
 * carries the face only: tool results, transcripts and artifacts stay in the
 * channel as signed events, so the avatar speaks while Buzz keeps the evidence.
 */
export function AgentMediaSurface({
  agentLabel,
  className,
  session,
}: AgentMediaSurfaceProps) {
  const {
    status,
    error,
    attachVideo,
    micEnabled,
    setMicEnabled,
    canPublishAudio,
  } = useAgentMediaRoom(session);

  return (
    <section
      aria-label={`${agentLabel} live video`}
      className={cn("flex flex-col gap-2", className)}
      data-testid="agent-media-surface"
    >
      <div className="relative flex aspect-video w-full items-center justify-center overflow-hidden rounded-lg bg-muted">
        {status === "error" ? (
          <div
            className="flex flex-col items-center gap-2 px-6 text-center"
            data-testid="agent-media-error"
          >
            <CircleAlert className="size-6 text-destructive" />
            <span className="text-sm text-muted-foreground">
              {error ?? "Could not join the session."}
            </span>
          </div>
        ) : attachVideo ? (
          <AgentVideoElement attachVideo={attachVideo} label={agentLabel} />
        ) : status === "connected" ? (
          <div
            className="flex flex-col items-center gap-2 text-muted-foreground"
            data-testid="agent-media-no-video"
          >
            <VideoOff className="size-6" />
            <span className="text-sm">No video on this session yet.</span>
          </div>
        ) : (
          <div
            className="flex flex-col items-center gap-2 text-muted-foreground"
            data-testid="agent-media-connecting"
          >
            <Spinner />
            <span className="text-sm">
              {status === "authorizing" ? "Authorizing…" : "Connecting…"}
            </span>
          </div>
        )}

        {canPublishAudio ? (
          <Button
            aria-label={micEnabled ? "Mute microphone" : "Unmute microphone"}
            className="absolute bottom-2 right-2"
            data-testid="agent-media-mic-toggle"
            disabled={status !== "connected"}
            onClick={() => setMicEnabled(!micEnabled)}
            size="icon"
            title={micEnabled ? "Mute microphone" : "Unmute microphone"}
            type="button"
            variant="secondary"
          >
            {micEnabled ? <Mic /> : <MicOff />}
          </Button>
        ) : null}
      </div>
      <p className="text-2xs text-muted-foreground">
        AI-generated avatar and voice.
      </p>
    </section>
  );
}
