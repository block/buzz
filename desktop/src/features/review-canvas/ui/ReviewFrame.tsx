import * as React from "react";

import { cn } from "@/shared/lib/cn";
import {
  buildInitMessage,
  type ChromeMessage,
  parseAnnotatorMessage,
} from "../lib/reviewAnnotatorProtocol";
import type { ReviewBlock } from "../lib/reviewContract";

/** How long the annotator has to announce itself before chrome takes over. */
const ANNOTATOR_READY_TIMEOUT_MS = 5_000;

export type ReviewFrameStatus =
  /** Waiting for the document and its annotator. */
  | "loading"
  /** The annotator handed over its port; the frame can report selections. */
  | "ready"
  /** The annotator never started; block selection works from chrome only. */
  | "degraded"
  /** The document tried to navigate itself and was removed. */
  | "blocked";

export type ReviewFrameHandle = {
  /** Move keyboard focus to a block inside the frame. */
  focusBlock: (id: string) => boolean;
  /** Current rendered size, recorded as feedback context. */
  measure: () => { width: number; height: number };
};

type ReviewFrameProps = {
  /** Accessible name of the frame. */
  title: string;
  /** Complete frame document (CSP meta first, annotator script last). */
  srcDoc: string;
  blocks: ReviewBlock[];
  selectedBlockId: string | null;
  onSelectBlock: (id: string) => void;
  onStatusChange?: (status: ReviewFrameStatus) => void;
  className?: string;
};

/**
 * The untrusted-document boundary. The factory HTML runs in a sandboxed,
 * opaque-origin frame (`sandbox="allow-scripts"` only, so no same-origin
 * access, forms, popups, top navigation, modals, or downloads) whose own CSP
 * allows a single nonce-bearing script: the annotator. The annotator reaches
 * this chrome only through a MessagePort transferred once at load.
 *
 * A second `load` event means the document navigated itself away from the
 * composed `srcdoc`; the frame is dropped rather than left running content
 * Buzz never verified.
 */
export const ReviewFrame = React.forwardRef<
  ReviewFrameHandle,
  ReviewFrameProps
>(function ReviewFrame(
  {
    title,
    srcDoc,
    blocks,
    selectedBlockId,
    onSelectBlock,
    onStatusChange,
    className,
  },
  ref,
) {
  const iframeRef = React.useRef<HTMLIFrameElement>(null);
  const portRef = React.useRef<MessagePort | null>(null);
  const loadCountRef = React.useRef(0);
  const readyTimerRef = React.useRef<number | null>(null);
  const [status, setStatus] = React.useState<ReviewFrameStatus>("loading");
  const knownIds = React.useMemo(
    () => new Set(blocks.map((block) => block.id)),
    [blocks],
  );

  // Latest callbacks without re-running the one-time handshake.
  const onSelectRef = React.useRef(onSelectBlock);
  onSelectRef.current = onSelectBlock;
  const onStatusRef = React.useRef(onStatusChange);
  onStatusRef.current = onStatusChange;

  const updateStatus = React.useCallback((next: ReviewFrameStatus) => {
    setStatus(next);
    onStatusRef.current?.(next);
  }, []);

  React.useImperativeHandle(
    ref,
    () => ({
      focusBlock: (id) => {
        const frame = iframeRef.current;
        const port = portRef.current;
        if (status !== "ready" || !frame || !port) return false;
        frame.focus();
        const message: ChromeMessage = { type: "focus", id };
        port.postMessage(message);
        return true;
      },
      measure: () => {
        const rect = iframeRef.current?.getBoundingClientRect();
        return { width: rect?.width ?? 1, height: rect?.height ?? 1 };
      },
    }),
    [status],
  );

  const clearReadyTimer = React.useCallback(() => {
    if (readyTimerRef.current !== null) {
      window.clearTimeout(readyTimerRef.current);
      readyTimerRef.current = null;
    }
  }, []);

  const handleLoad = React.useCallback(() => {
    loadCountRef.current += 1;
    if (loadCountRef.current > 1) {
      clearReadyTimer();
      portRef.current?.close();
      portRef.current = null;
      updateStatus("blocked");
      return;
    }
    const frameWindow = iframeRef.current?.contentWindow;
    if (!frameWindow) return;
    const channel = new MessageChannel();
    channel.port1.onmessage = (event) => {
      const message = parseAnnotatorMessage(event.data, knownIds);
      if (message?.type === "ready") {
        clearReadyTimer();
        updateStatus("ready");
      } else if (message?.type === "select") {
        onSelectRef.current(message.id);
      }
    };
    portRef.current = channel.port1;
    // An opaque-origin frame can only be addressed with "*"; the port, not
    // the origin, is the capability.
    frameWindow.postMessage(buildInitMessage(blocks), "*", [channel.port2]);
    readyTimerRef.current = window.setTimeout(() => {
      updateStatus("degraded");
    }, ANNOTATOR_READY_TIMEOUT_MS);
  }, [blocks, clearReadyTimer, knownIds, updateStatus]);

  React.useEffect(
    () => () => {
      clearReadyTimer();
      portRef.current?.close();
      portRef.current = null;
    },
    [clearReadyTimer],
  );

  React.useEffect(() => {
    if (status !== "ready") return;
    const message: ChromeMessage = { type: "selection", id: selectedBlockId };
    portRef.current?.postMessage(message);
  }, [selectedBlockId, status]);

  if (status === "blocked") {
    return (
      <div
        className={cn(
          "flex h-full items-center justify-center p-6 text-sm text-destructive",
          className,
        )}
        data-testid="review-canvas-frame-blocked"
        role="alert"
      >
        This review document tried to navigate away from its frame, so it was
        removed. Reload the review to try again.
      </div>
    );
  }

  return (
    <iframe
      allow=""
      className={cn("h-full w-full border-0 bg-white", className)}
      data-status={status}
      data-testid="review-canvas-frame"
      onLoad={handleLoad}
      ref={iframeRef}
      referrerPolicy="no-referrer"
      sandbox="allow-scripts"
      srcDoc={srcDoc}
      title={title}
    />
  );
});
