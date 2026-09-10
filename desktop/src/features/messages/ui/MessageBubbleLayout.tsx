import * as React from "react";
import { cn } from "@/shared/lib/cn";
import "./MessageBubbleLayout.css";
import { BubbleAvatar } from "./BubbleAvatarScope";

/** Present the existing message content and controls as a directional chat bubble. */
export function MessageBubbleLayout({
  messageId,
  showAvatar = true,
  outgoing,
  continuation,
  avatar,
  header,
  body,
  metadata,
  extras,
  footer,
  reactions,
  actions,
}: {
  messageId?: string;
  showAvatar?: boolean;
  outgoing: boolean;
  continuation: boolean;
  avatar: React.ReactNode;
  header: React.ReactNode;
  body: React.ReactNode;
  metadata: React.ReactNode;
  extras: React.ReactNode;
  footer?: React.ReactNode;
  reactions?: React.ReactNode;
  actions?: React.ReactNode;
}) {
  const anchorRef = React.useRef<HTMLDivElement>(null);
  const actionsRef = React.useRef<HTMLDivElement>(null);
  const [actionShift, setActionShift] = React.useState(0);
  const hasActions = Boolean(actions);
  React.useLayoutEffect(() => {
    if (!hasActions) return;
    const anchor = anchorRef.current;
    const controls = actionsRef.current;
    const row = anchor?.closest("article");
    if (!anchor || !controls || !row) return;
    const update = () => {
      const left =
        anchor.getBoundingClientRect().right + 8 - controls.offsetWidth;
      setActionShift(Math.max(0, row.getBoundingClientRect().left + 8 - left));
    };
    const observer = new ResizeObserver(update);
    observer.observe(anchor);
    observer.observe(controls);
    observer.observe(row);
    update();
    return () => observer.disconnect();
  }, [hasActions]);
  return (
    <>
      {!outgoing && <div className="w-7 shrink-0" />}
      <div
        className={cn(
          "flex min-w-0 max-w-[82%] flex-col",
          outgoing ? "ml-auto items-end" : "items-start",
        )}
      >
        {!continuation && (
          <div className="mb-1.5 px-1 text-xs text-muted-foreground">
            {outgoing ? (
              <>
                <span className="sr-only">You</span>
                {metadata}
              </>
            ) : (
              header
            )}
          </div>
        )}
        <div
          className={cn(
            "message-bubble-anchor relative min-w-0 max-w-full",
            footer && "w-full",
          )}
          ref={anchorRef}
          data-testid="message-bubble-anchor"
          data-bubble-direction={outgoing ? "outgoing" : "incoming"}
        >
          <div
            data-testid="message-body"
            data-message-bubble={outgoing ? "outgoing" : "incoming"}
            className={cn(
              "message-bubble-body min-w-0 max-w-full break-words text-message leading-relaxed [&_.message-markdown>p:first-child]:mt-0 [&_.message-markdown>p:last-child]:mb-0",
              footer && "w-full",
            )}
          >
            {body}
          </div>
          {!outgoing && showAvatar && (
            <div className="absolute -left-[38px] bottom-0 w-7 [&_[data-testid=message-avatar]]:!h-7 [&_[data-testid=message-avatar]]:!w-7">
              <BubbleAvatar messageId={messageId}>{avatar}</BubbleAvatar>
            </div>
          )}
          {reactions && (
            <div
              data-testid="bubble-reactions-anchor"
              className={cn(
                "absolute top-0 z-10 w-max max-w-[min(28rem,70vw)] -translate-y-1/2 [&_button]:text-badge [&_[data-testid=message-reactions]]:mt-0",
                outgoing
                  ? "-left-2 [&_[data-testid=message-reactions]]:justify-start"
                  : "-right-2 [&_[data-testid=message-reactions]]:justify-end",
              )}
            >
              {reactions}
            </div>
          )}
          {actions && (
            <div
              ref={actionsRef}
              style={{ right: -8 - actionShift }}
              data-testid="bubble-actions-anchor"
              className={cn(
                "absolute -right-2 z-20",
                reactions ? "bottom-full mb-5" : "top-0 -translate-y-1/2",
              )}
            >
              {actions}
            </div>
          )}
        </div>
        <div className="max-w-full">{extras}</div>
        {footer && <div className="w-full self-start">{footer}</div>}
      </div>
    </>
  );
}
