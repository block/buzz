import { isTauri } from "@tauri-apps/api/core";
import * as React from "react";

import { invokeTauri } from "@/shared/api/tauri";

type RemoteEmbedFrameProps = {
  src: string;
  title: string;
  testId: string;
  onLoad?: React.ReactEventHandler<HTMLIFrameElement>;
};

type EmbedRect = {
  x: number;
  y: number;
  width: number;
  height: number;
};

function hostRect(el: HTMLElement): EmbedRect | null {
  const rect = el.getBoundingClientRect();
  if (rect.width < 8 || rect.height < 8) return null;
  return {
    x: rect.left,
    y: rect.top,
    width: rect.width,
    height: rect.height,
  };
}

/**
 * Desktop: native child webview (HTML iframes stay blank in the transparent
 * macOS WKWebView). Web: iframe, which Chrome paints.
 */
export function RemoteEmbedFrame({
  src,
  title,
  testId,
  onLoad,
}: RemoteEmbedFrameProps) {
  const hostRef = React.useRef<HTMLDivElement>(null);
  const desktop = isTauri();

  React.useEffect(() => {
    if (!desktop) return;
    const el = hostRef.current;
    if (!el) return;
    let cancelled = false;

    const show = () => {
      if (cancelled) return;
      const rect = hostRect(el);
      if (!rect) return;
      void invokeTauri("show_channel_embed", { url: src, ...rect }).catch(
        (error) => {
          console.warn("show_channel_embed failed", error);
        },
      );
    };
    const move = () => {
      if (cancelled) return;
      const rect = hostRect(el);
      if (!rect) return;
      void invokeTauri("set_channel_embed_bounds", rect).catch(() => {});
    };

    show();
    const observer = new ResizeObserver(move);
    observer.observe(el);
    window.addEventListener("resize", move);
    return () => {
      cancelled = true;
      observer.disconnect();
      window.removeEventListener("resize", move);
      void invokeTauri("hide_channel_embed").catch(() => {});
    };
  }, [desktop, src]);

  if (!desktop) {
    return (
      <div className="relative min-h-0 flex-1 bg-background">
        <iframe
          className="absolute inset-0 h-full w-full border-0 bg-background"
          data-testid={testId}
          onLoad={onLoad}
          src={src}
          title={title}
        />
      </div>
    );
  }

  return (
    <div
      className="relative min-h-0 flex-1 bg-background"
      data-testid={testId}
      ref={hostRef}
    />
  );
}
