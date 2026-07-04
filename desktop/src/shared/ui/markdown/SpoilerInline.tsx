import * as React from "react";

import { hasBlockMedia } from "../markdownUtils";
import { SpoilerParticles } from "../SpoilerParticles";

export function SpoilerInline({
  block = false,
  children,
  interactive = true,
}: {
  block?: boolean;
  children?: React.ReactNode;
  interactive?: boolean;
}) {
  const [revealed, setRevealed] = React.useState(false);
  const contentRef = React.useRef<HTMLElement | null>(null);
  const isBlock = block || hasBlockMedia(React.Children.toArray(children));

  const setContentElement = React.useCallback((node: HTMLElement | null) => {
    contentRef.current = node;
  }, []);

  const toggleRevealed = React.useCallback(() => {
    setRevealed((value) => !value);
  }, []);

  const handlePointerDownCapture = React.useCallback(
    (event: React.PointerEvent<HTMLElement>) => {
      if (revealed) return;
      event.stopPropagation();
    },
    [revealed],
  );

  const handleClickCapture = React.useCallback(
    (event: React.MouseEvent<HTMLElement>) => {
      if (revealed) return;
      event.preventDefault();
      event.stopPropagation();
      toggleRevealed();
    },
    [revealed, toggleRevealed],
  );

  const handleClick = React.useCallback(
    (event: React.MouseEvent<HTMLElement>) => {
      if (revealed && isBlock && event.target !== event.currentTarget) return;
      toggleRevealed();
    },
    [isBlock, revealed, toggleRevealed],
  );

  const handleKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLElement>) => {
      if (event.key !== "Enter" && event.key !== " ") return;
      event.preventDefault();
      toggleRevealed();
    },
    [toggleRevealed],
  );

  const revealProps = {
    "aria-label": revealed ? "Hide spoiler" : "Reveal spoiler",
    "aria-pressed": revealed,
    onClick: handleClick,
    onClickCapture: handleClickCapture,
    onKeyDown: handleKeyDown,
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
            {children}
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
          {children}
        </span>
      </span>
    );
  }

  if (isBlock) {
    return (
      <div
        {...revealProps}
        className="buzz-spoiler buzz-spoiler--block"
        data-revealed={revealed ? "true" : "false"}
        data-spoiler=""
      >
        <SpoilerParticles active={!revealed} contentRef={contentRef} />
        <div className="buzz-spoiler__content" ref={setContentElement}>
          {children}
        </div>
      </div>
    );
  }

  return (
    <span
      {...revealProps}
      className="buzz-spoiler"
      data-revealed={revealed ? "true" : "false"}
      data-spoiler=""
    >
      <SpoilerParticles active={!revealed} contentRef={contentRef} />
      <span className="buzz-spoiler__content" ref={setContentElement}>
        {children}
      </span>
    </span>
  );
}
