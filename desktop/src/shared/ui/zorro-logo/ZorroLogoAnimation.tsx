import type { CSSProperties } from "react";
import { useId } from "react";
import "./zorro-logo-animation.css";

type VariantKey = "v1" | "v2" | "v3" | "v4" | "v5" | "v6" | "v7" | "v8";

export type ZorroLogoAnimationProps = {
  ariaLabel?: string;
  className?: string;
  fullScreen?: boolean;
  loop?: boolean;
  loopRestSeconds?: number;
  reverse?: boolean;
  showBackground?: boolean;
  style?: CSSProperties;
  /** When false, skips the animated texture filter. */
  textured?: boolean;
  /** Retained while callers migrate from the former mark animation API. */
  variant?: VariantKey;
};

type ZorroAnimationStyle = CSSProperties & {
  "--zorro-logo-cycle"?: string;
};

/** Animated Zorro mark used by onboarding and agent activity surfaces. */
export default function ZorroLogoAnimation({
  ariaLabel = "Zorro logo animation",
  className = "",
  fullScreen = true,
  loop = false,
  loopRestSeconds = 0,
  reverse = false,
  showBackground = true,
  style,
  textured = true,
}: ZorroLogoAnimationProps) {
  const idSuffix = useId().replace(/[^a-zA-Z0-9_-]/g, "");
  const textureId = `zorro-logo-texture-${idSuffix}`;
  const cycleSeconds = Math.max(1.1 + Math.max(loopRestSeconds, 0), 1.1);
  const animationStyle: ZorroAnimationStyle = {
    ...style,
    "--zorro-logo-cycle": `${cycleSeconds}s`,
  };
  const classes = [
    "zorro-logo",
    fullScreen && "zorro-logo--screen",
    !fullScreen && "zorro-logo--compact",
    showBackground && "zorro-logo--background",
    loop && "zorro-logo--loop",
    reverse && "zorro-logo--reverse",
    className,
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div
      aria-label={ariaLabel}
      className={classes}
      role="img"
      style={animationStyle}
    >
      <svg
        aria-hidden="true"
        className="zorro-logo__mark"
        height="512"
        viewBox="0 0 512 512"
        width="512"
      >
        {textured ? (
          <defs>
            <filter id={textureId} x="-15%" y="-15%" width="130%" height="130%">
              <feTurbulence
                baseFrequency="0.018"
                numOctaves="3"
                seed="17"
                type="fractalNoise"
              />
              <feDisplacementMap in="SourceGraphic" scale="9" />
            </filter>
          </defs>
        ) : null}
        <g
          className="zorro-logo__ink"
          filter={textured ? `url(#${textureId})` : undefined}
        >
          <path
            d="M39 346c31-38 132-59 217-59s186 21 217 59c18 22 7 46-25 59-49 20-120 31-192 31S113 425 64 405c-32-13-43-37-25-59Z"
            fill="#151112"
          />
          <path
            d="M60 344c42-27 119-42 196-42s154 15 196 42c20 13 19 27-2 39-42 24-119 38-194 38S104 407 62 383c-21-12-22-26-2-39Z"
            fill="#2a2325"
          />
          <path
            d="M174 216c0-20 37-33 82-33s82 13 82 33l14 132H160l14-132Z"
            fill="#211b1d"
          />
          <path
            d="M174 216c0-20 37-33 82-33s82 13 82 33c0 20-37 33-82 33s-82-13-82-33Z"
            fill="#3a3033"
          />
          <path
            d="M220 220h72v22l-42 34h42v24h-72v-23l42-34h-42v-23Z"
            fill="#f6cacc"
          />
          <path
            d="M73 374c50 20 116 30 183 30s133-10 183-30c-30 30-106 48-183 48S103 404 73 374Z"
            fill="#0d0a0b"
            opacity="0.55"
          />
        </g>
      </svg>
    </div>
  );
}
