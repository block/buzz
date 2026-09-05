import * as React from "react";

import { hasBlockMedia } from "../markdownUtils";
import { SpoilerParticles } from "../SpoilerParticles";

/**
 * True for descendants of a spoiler that is currently hidden. Consumers (e.g.
 * `MaskedLinkTooltip`) use it to suppress hover/focus affordances that would
 * otherwise leak masked content before the spoiler is revealed. Default
 * `false` — content outside any spoiler is never hidden.
 */
export const SpoilerHiddenContext = React.createContext(false);

export function SpoilerInline({
  block = false,
  children,
  interactive = true,
  revealOnHover = false,
}: {
  block?: boolean;
  children?: React.ReactNode;
  interactive?: boolean;
  /** Temporarily reveal for mouse/trackpad hover while preserving click and keyboard access. */
  revealOnHover?: boolean;
}) {
  const [revealed, setRevealed] = React.useState(false);
  const [hovered, setHovered] = React.useState(false);
  const contentRef = React.useRef<HTMLElement | null>(null);
  const isBlock = block || hasBlockMedia(React.Children.toArray(children));
  const visuallyRevealed = revealed || hovered;

  const setContentElement = React.useCallback((node: HTMLElement | null) => {
    contentRef.current = node;
  }, []);

  const toggleRevealed = React.useCallback(() => {
    setRevealed((value) => !value);
  }, []);

  const handlePointerDownCapture = React.useCallback(
    (event: React.PointerEvent<HTMLElement>) => {
      if (visuallyRevealed) return;
      event.stopPropagation();
    },
    [visuallyRevealed],
  );

  const handleClickCapture = React.useCallback(
    (event: React.MouseEvent<HTMLElement>) => {
      if (visuallyRevealed) return;
      event.preventDefault();
      event.stopPropagation();
      toggleRevealed();
    },
    [toggleRevealed, visuallyRevealed],
  );

  const handleClick = React.useCallback(
    (event: React.MouseEvent<HTMLElement>) => {
      // Hover is a temporary reveal: clicking or selecting while the pointer
      // is inside must not pin the spoiler open after the pointer leaves.
      // Touch and keyboard still toggle `revealed` because they have no hover.
      if (revealOnHover && hovered) return;
      if (visuallyRevealed && isBlock && event.target !== event.currentTarget)
        return;
      toggleRevealed();
    },
    [hovered, isBlock, revealOnHover, toggleRevealed, visuallyRevealed],
  );

  const handleKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLElement>) => {
      if (event.key !== "Enter" && event.key !== " ") return;
      event.preventDefault();
      toggleRevealed();
    },
    [toggleRevealed],
  );

  const handlePointerEnter = React.useCallback(
    (event: React.PointerEvent<HTMLElement>) => {
      if (revealOnHover && event.pointerType === "mouse") setHovered(true);
    },
    [revealOnHover],
  );

  const handlePointerLeave = React.useCallback(() => {
    if (revealOnHover) setHovered(false);
  }, [revealOnHover]);

  const revealProps = {
    "aria-label": revealed ? "Hide spoiler" : "Reveal spoiler",
    "aria-pressed": revealed,
    onClick: handleClick,
    onClickCapture: handleClickCapture,
    onKeyDown: handleKeyDown,
    onPointerEnter: handlePointerEnter,
    onPointerLeave: handlePointerLeave,
    onPointerDownCapture: handlePointerDownCapture,
    role: "button",
    tabIndex: 0,
  } as const;

  if (!interactive) {
    if (isBlock) {
      return (
        <div
          className="buzz-spoiler buzz-spoiler--block buzz-spoiler--inert"
          data-revealed="false"
          data-spoiler=""
        >
          <SpoilerParticles active contentRef={contentRef} />
          <div className="buzz-spoiler__content" ref={setContentElement}>
            <SpoilerHiddenContext.Provider value={true}>
              {children}
            </SpoilerHiddenContext.Provider>
          </div>
        </div>
      );
    }

    return (
      <span
        className="buzz-spoiler buzz-spoiler--inert"
        data-revealed="false"
        data-spoiler=""
      >
        <SpoilerParticles active contentRef={contentRef} />
        <span className="buzz-spoiler__content" ref={setContentElement}>
          <SpoilerHiddenContext.Provider value={true}>
            {children}
          </SpoilerHiddenContext.Provider>
        </span>
      </span>
    );
  }

  if (isBlock) {
    return (
      <div
        {...revealProps}
        className="buzz-spoiler buzz-spoiler--block"
        data-revealed={visuallyRevealed ? "true" : "false"}
        data-spoiler=""
      >
        <SpoilerParticles active={!visuallyRevealed} contentRef={contentRef} />
        <div className="buzz-spoiler__content" ref={setContentElement}>
          <SpoilerHiddenContext.Provider value={!visuallyRevealed}>
            {children}
          </SpoilerHiddenContext.Provider>
        </div>
      </div>
    );
  }

  return (
    <span
      {...revealProps}
      className="buzz-spoiler"
      data-revealed={visuallyRevealed ? "true" : "false"}
      data-spoiler=""
    >
      <SpoilerParticles active={!visuallyRevealed} contentRef={contentRef} />
      <span className="buzz-spoiler__content" ref={setContentElement}>
        <SpoilerHiddenContext.Provider value={!visuallyRevealed}>
          {children}
        </SpoilerHiddenContext.Provider>
      </span>
    </span>
  );
}
