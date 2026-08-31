import * as React from "react";
import { CircleAlert, Mic, MicOff, VideoOff, Volume2 } from "lucide-react";

import { attachSessionTracks } from "@/features/agents/lib/agentMediaAttach";
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

/**
 * Attach the agent's tracks to one `<video>` for as long as they exist.
 *
 * **Both tracks go on the same element, deliberately.** LiveKit's `attach`
 * merges a track into whatever `MediaStream` the element already carries, so one
 * element means one stream, one jitter buffer and one playback clock — which is
 * what keeps the voice on the lips. Two elements play from two independent
 * clocks and drift audibly.
 *
 * It is also how the element ends up audible at all: `attachToElement` sets
 * `element.muted = mediaStream.getAudioTracks().length === 0`, so a video-only
 * stream is muted by the SDK no matter what the markup says.
 */
function AgentMediaElement({
  attachAudio,
  attachVideo,
  label,
}: {
  attachAudio: ((element: HTMLMediaElement) => () => void) | null;
  attachVideo: (element: HTMLVideoElement) => () => void;
  label: string;
}) {
  const ref = React.useRef<HTMLVideoElement | null>(null);

  React.useEffect(() => {
    const element = ref.current;
    if (!element) return;
    return attachSessionTracks(element, { attachAudio, attachVideo });
  }, [attachAudio, attachVideo]);

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
      playsInline
      ref={ref}
    />
  );
}

/**
 * Play the agent's audio when there is no video to carry it.
 *
 * A session may announce audio only, or its video may drop while the call
 * continues. Hidden, because there is nothing to look at and a second set of
 * media controls would invite muting the agent by way of a control that looks
 * like it belongs to the tile.
 */
function AgentAudioOnlyElement({
  attachAudio,
  label,
}: {
  attachAudio: (element: HTMLMediaElement) => () => void;
  label: string;
}) {
  const ref = React.useRef<HTMLAudioElement | null>(null);

  React.useEffect(() => {
    const element = ref.current;
    if (!element) return;
    return attachAudio(element);
  }, [attachAudio]);

  return (
    // Same live-stream exemption as the video element above.
    // biome-ignore lint/a11y/useMediaCaption: live stream, see above
    <audio
      aria-label={`${label} audio`}
      autoPlay
      className="hidden"
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
    attachAudio,
    audioBlocked,
    enableAudio,
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
          <AgentMediaElement
            attachAudio={attachAudio}
            attachVideo={attachVideo}
            label={agentLabel}
          />
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

        {attachAudio && !attachVideo ? (
          <AgentAudioOnlyElement attachAudio={attachAudio} label={agentLabel} />
        ) : null}

        {audioBlocked ? (
          <Button
            className="absolute bottom-2 left-2"
            data-testid="agent-media-enable-audio"
            onClick={enableAudio}
            size="sm"
            type="button"
            variant="secondary"
          >
            <Volume2 />
            Enable sound
          </Button>
        ) : null}

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
