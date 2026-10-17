import JMuxer from "jmuxer";
import { RotateCw } from "lucide-react";
import * as React from "react";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import type { ScreenVideoStream, VideoBatch } from "../lib/screenShare";

/** H.264 is transmuxed to live MP4; the browser's video engine renders every frame. */
export function ScreenVideoPlayer({
  stream,
  label,
}: {
  stream: ScreenVideoStream;
  label: string;
}) {
  const ref = React.useRef<HTMLVideoElement>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [retry, setRetry] = React.useState(0);
  // biome-ignore lint/correctness/useExhaustiveDependencies: retry intentionally rebuilds failed muxer/decoder resources.
  React.useEffect(() => {
    const video = ref.current;
    if (!video) return;
    let disposed = false;
    let failed = false;
    setError(null);
    let muxer: JMuxer | null = null;
    let waitingForKey = true;
    let codec = "";
    let ready = false;
    let generation = 0;
    let pending: VideoBatch[] = [];
    let pendingBytes = 0;
    const destroy = () => {
      generation++;
      ready = false;
      pending = [];
      pendingBytes = 0;
      const url = video.src;
      muxer?.destroy();
      muxer = null;
      video.pause();
      video.removeAttribute("src");
      video.load();
      if (url.startsWith("blob:")) URL.revokeObjectURL(url);
    };
    const fail = (message: string) => {
      if (disposed) return;
      failed = true;
      setError(message);
      destroy();
      waitingForKey = true;
    };
    const feed = (batch: VideoBatch | null) => {
      if (disposed) return;
      if (!batch) {
        destroy();
        waitingForKey = true;
        return;
      }
      if (failed) return;
      let frames = batch.frames;
      if (waitingForKey) {
        const key = frames.findIndex((frame) => frame.key);
        if (key < 0) return;
        frames = frames.slice(key);
      }
      try {
        if (!muxer || codec !== batch.codec) {
          destroy();
          codec = batch.codec;
          if (
            !window.MediaSource?.isTypeSupported(`video/mp4; codecs="${codec}"`)
          ) {
            fail("This webview cannot decode the shared H.264 video.");
            return;
          }
          const epoch = generation;
          muxer = new JMuxer({
            node: video,
            mode: "video",
            fps: 30,
            flushingTime: 50,
            maxDelay: 800,
            clearBuffer: true,
            onReady: () => {
              if (disposed || epoch !== generation) return;
              ready = true;
              const buffered = pending;
              pending = [];
              pendingBytes = 0;
              for (const packet of buffered) feed(packet);
            },
            onError: () => {
              if (epoch === generation)
                fail("Shared video could not be decoded.");
            },
            onUnsupportedCodec: () => {
              if (epoch === generation)
                fail("This webview cannot decode the shared video.");
            },
          });
        }
        if (!ready) {
          const packet = { ...batch, frames };
          pendingBytes += frames.reduce(
            (size, frame) => size + frame.data.length,
            0,
          );
          if (pending.length >= 12 || pendingBytes > 2 * 1024 * 1024) {
            fail("Video playback did not become ready.");
            return;
          }
          pending.push(packet);
          waitingForKey = false;
          return;
        }
        waitingForKey = false;
        setError(null);
        for (let i = 0; i < frames.length; i++) {
          const frame = frames[i];
          const binary = atob(frame.data);
          const data = Uint8Array.from(binary, (byte) => byte.charCodeAt(0));
          const next =
            frames[i + 1]?.timestamp ??
            frame.timestamp +
              (i > 0
                ? frame.timestamp - frames[i - 1].timestamp
                : 1_000_000 / 30);
          muxer.feed({
            video: data,
            duration: Math.max(
              1,
              Math.round(next / 1000) - Math.round(frame.timestamp / 1000),
            ),
            isLastVideoFrameComplete: true,
          });
        }
        // Bound playback lag; don't retain minutes of video in a paused viewer.
        if (
          video.buffered.length &&
          video.buffered.end(video.buffered.length - 1) - video.currentTime > 3
        ) {
          video.currentTime = Math.max(
            0,
            video.buffered.end(video.buffered.length - 1) - 0.5,
          );
        }
        const epoch = generation;
        void video.play().catch((playError) => {
          if (!disposed && epoch === generation)
            setError(`Video playback failed: ${String(playError)}`);
        });
      } catch (error) {
        fail(`Video playback failed: ${String(error)}`);
      }
    };
    const unsubscribe = stream.subscribe(feed);
    return () => {
      disposed = true;
      unsubscribe();
      destroy();
    };
  }, [stream, retry]);
  return (
    <>
      <video
        ref={ref}
        aria-label={label}
        className="h-full w-full object-contain"
        autoPlay
        muted
        playsInline
        data-testid="huddle-shared-screen"
      />
      {error ? (
        <div className="absolute inset-x-0 bottom-0 flex items-center gap-2 bg-black/80 px-3 py-2">
          <p
            role="alert"
            className="min-w-0 flex-1 break-words text-sm text-white"
          >
            {error}
          </p>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                aria-label="Retry video playback"
                size="icon"
                variant="ghost"
                onClick={() => setRetry((value) => value + 1)}
              >
                <RotateCw className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Retry video playback</TooltipContent>
          </Tooltip>
        </div>
      ) : null}
    </>
  );
}
