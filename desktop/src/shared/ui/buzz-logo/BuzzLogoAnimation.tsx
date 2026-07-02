import { useId, useLayoutEffect, useRef } from "react";
import type { CSSProperties } from "react";
import "./buzz-logo-animation.css";

const LOOP = "indefinite";
const EASE = ".16 1 .3 1";

type TextureKey = "soft" | "fuzzy";
type VariantKey = "v1" | "v2" | "v3" | "v4" | "v5" | "v6" | "v7" | "v8";
type Timing = [string, string];
type PartTimings = Record<string, Timing>;
type VariantConfig = {
  duration: string;
  texture?: TextureKey;
  body: PartTimings;
  leftEye: PartTimings;
  rightEye: PartTimings;
  topSlot: PartTimings;
  bottomSlot: PartTimings;
  leftSide: PartTimings;
  rightSide: PartTimings;
};

type TextureConfig = {
  blur: string;
  displacement: string;
  frequency: string;
  frequencyValues: string;
  grainAlpha: string;
  seedValues: string;
};

export type BuzzLogoAnimationProps = {
  ariaLabel?: string;
  className?: string;
  fullScreen?: boolean;
  loop?: boolean;
  /**
   * When looping, hide the mark for this many seconds between plays. The morph
   * runs at its native speed, then the mark disappears for the rest window
   * before the cycle repeats. Only applies when `loop` is true.
   */
  loopRestSeconds?: number;
  reverse?: boolean;
  showBackground?: boolean;
  style?: CSSProperties;
  /** When false, skips the looping feTurbulence texture filter (CPU-heavy). */
  textured?: boolean;
  variant?: VariantKey;
};
const TEXTURES: Record<TextureKey, TextureConfig> = {
  soft: {
    blur: "6.4",
    displacement: "7",
    frequency: "0.86",
    frequencyValues: "0.82;0.98;0.78;1.04;0.82",
    grainAlpha: ".34",
    seedValues: "4;13;7;19;4",
  },
  fuzzy: {
    blur: "9",
    displacement: "10",
    frequency: "1.06",
    frequencyValues: "1;1.18;0.96;1.24;1",
    grainAlpha: ".5",
    seedValues: "6;21;11;27;6",
  },
};

const VARIANTS: Record<string, VariantConfig> = {
  v1: {
    duration: "2s",
    body: {
      x: [
        "0;0.1;0.22;0.29;0.39;0.45;0.82;1",
        "186;186;124;128;128;128;128;128",
      ],
      y: ["0;0.1;0.22;0.29;0.39;0.45;0.82;1", "108;108;45;49;-5;0;0;0"],
      width: [
        "0;0.1;0.22;0.29;0.39;0.45;0.82;1",
        "93;93;218;210;210;210;210;210",
      ],
      height: [
        "0;0.1;0.22;0.29;0.39;0.45;0.82;1",
        "93;93;218;210;318;309;309;309",
      ],
      rx: ["0;0.1;0.22;0.29;0.39;0.45;0.82;1", "14;14;38;34;36;34;34;34"],
    },
    leftEye: {
      cx: ["0;0.42;0.5;0.55;0.82;1", "233.4;233.4;188.5;193.3;193.3;193.3"],
      cy: ["0;0.42;0.5;0.55;0.82;1", "154.5;154.5;80.5;84.4;84.4;84.4"],
    },
    rightEye: {
      cx: ["0;0.42;0.5;0.55;0.82;1", "233.4;233.4;280;276;276;276"],
      cy: ["0;0.42;0.5;0.55;0.82;1", "154.5;154.5;80.5;84.4;84.4;84.4"],
    },
    topSlot: {
      opacity: ["0;0.57;0.58;0.69;1", "0;0;1;1;1"],
      x: ["0;0.57;0.64;0.69;1", "234.8;234.8;162;166.3;166.3"],
      width: ["0;0.57;0.64;0.69;1", "0;0;146;136.9;136.9"],
    },
    bottomSlot: {
      opacity: ["0;0.72;0.73;0.86;1", "0;0;1;1;1"],
      x: ["0;0.72;0.8;0.86;1", "234.8;234.8;162;166.9;166.9"],
      width: ["0;0.72;0.8;0.86;1", "0;0;146;136.2;136.2"],
    },
    leftSide: {
      opacity: ["0;0.84;0.85;1", "0;0;1;1"],
      cx: ["0;0.78;0.88;0.94;1", "233;233;233;86;91.7"],
    },
    rightSide: {
      opacity: ["0;0.84;0.85;1", "0;0;1;1"],
      cx: ["0;0.78;0.88;0.94;1", "233;233;233;380;374.3"],
    },
  },
  v2: {
    duration: "1.9s",
    body: {
      x: ["0;0.16;0.33;0.4;0.52;0.6;1", "186;186;124;124;128;128;128"],
      y: ["0;0.16;0.33;0.4;0.52;0.6;1", "108;108;45;45;-6;0;0"],
      width: ["0;0.16;0.33;0.4;0.52;0.6;1", "93;93;218;218;210;210;210"],
      height: ["0;0.16;0.33;0.4;0.52;0.6;1", "93;93;218;218;318;309;309"],
      rx: ["0;0.16;0.33;0.4;0.52;0.6;1", "14;14;38;38;36;34;34"],
    },
    leftEye: {
      cx: ["0;0.47;0.64;0.7;1", "233.4;233.4;188.5;193.3;193.3"],
      cy: ["0;0.47;0.64;0.7;1", "154.5;154.5;80.5;84.4;84.4"],
    },
    rightEye: {
      cx: ["0;0.47;0.64;0.7;1", "233.4;233.4;280;276;276"],
      cy: ["0;0.47;0.64;0.7;1", "154.5;154.5;80.5;84.4;84.4"],
    },
    topSlot: {
      opacity: ["0;0.62;0.63;1", "0;0;1;1"],
      x: ["0;0.62;0.76;0.82;1", "234.8;234.8;162;166.3;166.3"],
      width: ["0;0.62;0.76;0.82;1", "0;0;146;136.9;136.9"],
    },
    bottomSlot: {
      opacity: ["0;0.72;0.73;1", "0;0;1;1"],
      x: ["0;0.72;0.86;0.92;1", "234.8;234.8;162;166.9;166.9"],
      width: ["0;0.72;0.86;0.92;1", "0;0;146;136.2;136.2"],
    },
    leftSide: {
      opacity: ["0;0.81;0.82;1", "0;0;1;1"],
      cx: ["0;0.82;0.94;1", "233;233;82;91.7"],
    },
    rightSide: {
      opacity: ["0;0.81;0.82;1", "0;0;1;1"],
      cx: ["0;0.82;0.94;1", "233;233;384;374.3"],
    },
  },
};

VARIANTS.v3 = { ...VARIANTS.v1, duration: "1.45s" };
VARIANTS.v4 = {
  duration: "0.95s",
  body: {
    x: ["0;0.18;0.31;0.42;1", "186;124;128;128;128"],
    y: ["0;0.18;0.31;0.42;1", "108;45;-6;0;0"],
    width: ["0;0.18;0.31;1", "93;218;210;210"],
    height: ["0;0.18;0.31;0.42;1", "93;218;318;309;309"],
    rx: ["0;0.18;0.31;0.42;1", "14;38;36;34;34"],
  },
  leftEye: {
    cx: ["0;0.3;0.52;0.62;1", "233.4;233.4;188.5;193.3;193.3"],
    cy: ["0;0.3;0.52;0.62;1", "154.5;154.5;80.5;84.4;84.4"],
  },
  rightEye: {
    cx: ["0;0.3;0.52;0.62;1", "233.4;233.4;280;276;276"],
    cy: ["0;0.3;0.52;0.62;1", "154.5;154.5;80.5;84.4;84.4"],
  },
  topSlot: {
    opacity: ["0;0.46;0.47;1", "0;0;1;1"],
    x: ["0;0.46;0.66;0.76;1", "234.8;234.8;162;166.3;166.3"],
    width: ["0;0.46;0.66;0.76;1", "0;0;146;136.9;136.9"],
  },
  bottomSlot: {
    opacity: ["0;0.58;0.59;1", "0;0;1;1"],
    x: ["0;0.58;0.76;0.86;1", "234.8;234.8;162;166.9;166.9"],
    width: ["0;0.58;0.76;0.86;1", "0;0;146;136.2;136.2"],
  },
  leftSide: {
    opacity: ["0;0.7;0.71;1", "0;0;1;1"],
    cx: ["0;0.7;0.9;1", "233;233;82;91.7"],
  },
  rightSide: {
    opacity: ["0;0.7;0.71;1", "0;0;1;1"],
    cx: ["0;0.7;0.9;1", "233;233;384;374.3"],
  },
};
VARIANTS.v5 = {
  duration: "0.88s",
  body: {
    opacity: ["0;0.03;0.08;1", "0;0;1;1"],
    x: [
      "0;0.08;0.16;0.23;0.35;0.47;0.6;1",
      "233.4;200;186;186;120;132;127;128",
    ],
    y: ["0;0.08;0.16;0.23;0.35;0.47;0.6;1", "154.5;121;108;108;42;-12;3;0"],
    width: ["0;0.08;0.16;0.23;0.35;0.47;0.6;1", "0;66;93;93;226;204;212;210"],
    height: ["0;0.08;0.16;0.23;0.35;0.47;0.6;1", "0;66;93;93;226;326;304;309"],
    rx: ["0;0.08;0.16;0.23;0.35;0.47;0.6;1", "0;33;46.5;14;40;37;33;34"],
  },
  leftEye: {
    opacity: ["0;0.18;0.2;1", "0;0;1;1"],
    cx: ["0;0.23;0.45;0.58;0.68;1", "233.4;233.4;185;196;193.3;193.3"],
    cy: ["0;0.23;0.45;0.58;0.68;1", "154.5;154.5;76;87;84.4;84.4"],
  },
  rightEye: {
    opacity: ["0;0.18;0.2;1", "0;0;1;1"],
    cx: ["0;0.23;0.45;0.58;0.68;1", "233.4;233.4;283;273;276;276"],
    cy: ["0;0.23;0.45;0.58;0.68;1", "154.5;154.5;76;87;84.4;84.4"],
  },
  topSlot: {
    opacity: ["0;0.42;0.43;1", "0;0;1;1"],
    x: ["0;0.42;0.6;0.72;0.82;1", "234.8;234.8;158;169;166.3;166.3"],
    width: ["0;0.42;0.6;0.72;0.82;1", "0;0;153;132;136.9;136.9"],
  },
  bottomSlot: {
    opacity: ["0;0.54;0.55;1", "0;0;1;1"],
    x: ["0;0.54;0.72;0.84;0.94;1", "234.8;234.8;158;169;166.9;166.9"],
    width: ["0;0.54;0.72;0.84;0.94;1", "0;0;153;132;136.2;136.2"],
  },
  leftSide: {
    opacity: ["0;0.66;0.67;1", "0;0;1;1"],
    cx: ["0;0.66;0.84;0.96;1", "233;233;76;95;91.7"],
  },
  rightSide: {
    opacity: ["0;0.66;0.67;1", "0;0;1;1"],
    cx: ["0;0.66;0.84;0.96;1", "233;233;390;371;374.3"],
  },
};

function scaleTiming([keyTimes, values]: Timing, scale: number): Timing {
  const times = keyTimes.split(";").map((time) => Number(time) * scale);
  const splitValues = values.split(";");

  const lastTime = times[times.length - 1];
  if (lastTime !== undefined && lastTime < 1) {
    times.push(1);
    const lastValue = splitValues[splitValues.length - 1];
    if (lastValue !== undefined) {
      splitValues.push(lastValue);
    }
  }

  return [
    times.map((time) => String(Number(time.toFixed(4)))).join(";"),
    splitValues.join(";"),
  ];
}

function scaleVariant(
  variant: VariantConfig,
  duration: string,
  scale: number,
): VariantConfig {
  return Object.fromEntries(
    Object.entries(variant).map(([part, timings]) => {
      if (part === "duration") {
        return [part, duration];
      }
      if (part === "texture") {
        return [part, timings];
      }

      return [
        part,
        Object.fromEntries(
          Object.entries(timings as PartTimings).map(([attribute, timing]) => [
            attribute,
            scaleTiming(timing, scale),
          ]),
        ),
      ];
    }),
  ) as VariantConfig;
}

function reverseTiming([keyTimes, values]: Timing): Timing {
  const times = keyTimes.split(";").map((time) => 1 - Number(time));
  const splitValues = values.split(";");
  const pairs = times
    .map((time, index) => ({ time, value: splitValues[index] }))
    .reverse();

  return [
    pairs.map(({ time }) => String(Number(time.toFixed(4)))).join(";"),
    pairs.map(({ value }) => value).join(";"),
  ];
}

function reverseVariant(variant: VariantConfig): VariantConfig {
  return Object.fromEntries(
    Object.entries(variant).map(([part, timings]) => {
      if (part === "duration" || part === "texture") {
        return [part, timings];
      }

      return [
        part,
        Object.fromEntries(
          Object.entries(timings as PartTimings).map(([attribute, timing]) => [
            attribute,
            reverseTiming(timing),
          ]),
        ),
      ];
    }),
  ) as VariantConfig;
}

VARIANTS.v6 = scaleVariant(VARIANTS.v5, "1.38s", 0.64);
VARIANTS.v6.leftEye.ry = [
  "0;0.64;0.72;0.78;0.84;0.9;0.96;1",
  "27;27;2;27;27;2;27;27",
];
VARIANTS.v6.rightEye.ry = VARIANTS.v6.leftEye.ry;
VARIANTS.v7 = {
  ...VARIANTS.v5,
  texture: "fuzzy",
  leftSide: {
    ...VARIANTS.v5.leftSide,
    opacity: ["0;0.54;0.55;1", "0;0;1;1"],
    cx: ["0;0.54;0.74;0.9;1", "233;233;76;95;91.7"],
  },
  rightSide: {
    ...VARIANTS.v5.rightSide,
    opacity: ["0;0.54;0.55;1", "0;0;1;1"],
    cx: ["0;0.54;0.74;0.9;1", "233;233;390;371;374.3"],
  },
};
VARIANTS.v8 = {
  ...VARIANTS.v7,
  leftEye: {
    ...VARIANTS.v7.leftEye,
    ry: ["0;0.76;0.84;0.92;1", "27;27;2;27;27"],
  },
  rightEye: {
    ...VARIANTS.v7.rightEye,
    ry: ["0;0.76;0.84;0.92;1", "27;27;2;27;27"],
  },
};

function splines(keyTimes: string) {
  return Array.from(
    { length: keyTimes.split(";").length - 1 },
    () => EASE,
  ).join(";");
}

function SvgAnimate({
  attributeName,
  duration,
  keyTimes,
  repeatCount,
  values,
}: {
  attributeName: string;
  duration: string;
  keyTimes: string;
  repeatCount: string;
  values: string;
}) {
  return (
    <animate
      attributeName={attributeName}
      begin="indefinite"
      dur={duration}
      calcMode="spline"
      fill={repeatCount === LOOP ? "remove" : "freeze"}
      keySplines={splines(keyTimes)}
      keyTimes={keyTimes}
      repeatCount={repeatCount}
      values={values}
    />
  );
}

function animationsFor(
  config: PartTimings,
  duration: string,
  repeatCount: string,
) {
  return Object.entries(config).map(([attributeName, [keyTimes, values]]) => (
    <SvgAnimate
      key={attributeName}
      attributeName={attributeName}
      duration={duration}
      keyTimes={keyTimes}
      repeatCount={repeatCount}
      values={values}
    />
  ));
}

function idPart(value: string) {
  return value.replace(/[^a-zA-Z0-9_-]/g, "");
}

function TextureFilter({
  id,
  texture,
}: {
  id: string;
  texture: TextureConfig;
}) {
  return (
    <filter
      id={id}
      x="-80"
      y="-80"
      width="626"
      height="469"
      filterUnits="userSpaceOnUse"
      colorInterpolationFilters="sRGB"
    >
      <feGaussianBlur
        in="SourceGraphic"
        stdDeviation={texture.blur}
        result="softLogo"
      />
      <feTurbulence
        type="fractalNoise"
        baseFrequency={texture.frequency}
        numOctaves="5"
        seed="7"
        result="textureNoise"
      >
        <animate
          attributeName="baseFrequency"
          begin="indefinite"
          dur="0.34s"
          repeatCount={LOOP}
          values={texture.frequencyValues}
        />
        <animate
          attributeName="seed"
          begin="indefinite"
          dur="0.34s"
          repeatCount={LOOP}
          values={texture.seedValues}
        />
      </feTurbulence>
      <feDisplacementMap
        in="softLogo"
        in2="textureNoise"
        scale={texture.displacement}
        xChannelSelector="R"
        yChannelSelector="G"
        result="buzzedLogo"
      />
      <feColorMatrix
        in="textureNoise"
        type="matrix"
        values={`0 0 0 0 0
                0 0 0 0 0
                0 0 0 0 0
                ${texture.grainAlpha} ${texture.grainAlpha} ${texture.grainAlpha} 0 0`}
        result="grainAlpha"
      />
      <feComposite
        in="buzzedLogo"
        in2="grainAlpha"
        operator="in"
        result="grainLogo"
      />
      <feMerge>
        <feMergeNode in="buzzedLogo" />
        <feMergeNode in="grainLogo" />
      </feMerge>
    </filter>
  );
}

function CutoutMask({
  config,
  duration,
  id,
  repeatCount,
}: {
  config: VariantConfig;
  duration: string;
  id: string;
  repeatCount: string;
}) {
  return (
    <mask
      id={id}
      x="-80"
      y="-80"
      width="626"
      height="469"
      maskUnits="userSpaceOnUse"
      maskContentUnits="userSpaceOnUse"
    >
      <rect x="-80" y="-80" width="626" height="469" fill="#fff" />
      <ellipse
        cx="233.4"
        cy="154.5"
        rx="27"
        ry="27"
        fill="#000"
        opacity={"opacity" in config.leftEye ? "0" : undefined}
      >
        {animationsFor(config.leftEye, duration, repeatCount)}
      </ellipse>
      <ellipse
        cx="233.4"
        cy="154.5"
        rx="27"
        ry="27"
        fill="#000"
        opacity={"opacity" in config.rightEye ? "0" : undefined}
      >
        {animationsFor(config.rightEye, duration, repeatCount)}
      </ellipse>
      <rect
        x="234.8"
        y="157.2"
        width="0"
        height="38.3"
        rx="5"
        fill="#000"
        opacity="0"
      >
        {animationsFor(config.topSlot, duration, repeatCount)}
      </rect>
      <rect
        x="234.8"
        y="235.1"
        width="0"
        height="37.6"
        rx="5"
        fill="#000"
        opacity="0"
      >
        {animationsFor(config.bottomSlot, duration, repeatCount)}
      </rect>
    </mask>
  );
}

function InkShapes({
  config,
  duration,
  repeatCount,
}: {
  config: VariantConfig;
  duration: string;
  repeatCount: string;
}) {
  return (
    <>
      <circle
        className="buzz-logo__ink"
        cx="233"
        cy="154.5"
        r="91.7"
        opacity="0"
      >
        {animationsFor(config.leftSide, duration, repeatCount)}
      </circle>
      <circle
        className="buzz-logo__ink"
        cx="233"
        cy="154.5"
        r="91.7"
        opacity="0"
      >
        {animationsFor(config.rightSide, duration, repeatCount)}
      </circle>

      <rect
        className="buzz-logo__ink"
        x="186"
        y="108"
        width="93"
        height="93"
        rx="14"
        opacity={"opacity" in config.body ? "0" : undefined}
      >
        {animationsFor(config.body, duration, repeatCount)}
      </rect>
    </>
  );
}

/**
 * Hides the parent group during the rest window of a stretched loop cycle:
 * fully visible while the morph plays, then a quick fade to invisible for the
 * remainder of the cycle. SMIL `<animate>` targets its parent element.
 */
function RestWindowFade({
  cycleSeconds,
  morphSeconds,
  repeatCount,
}: {
  cycleSeconds: number;
  morphSeconds: number;
  repeatCount: string;
}) {
  const fadeSeconds = 0.15;
  const visibleEnd = morphSeconds / cycleSeconds;
  const fadeEnd = Math.min((morphSeconds + fadeSeconds) / cycleSeconds, 1);
  const keyTimes = ["0", visibleEnd.toFixed(4), fadeEnd.toFixed(4), "1"].join(
    ";",
  );

  return (
    <animate
      attributeName="opacity"
      begin="indefinite"
      calcMode="linear"
      dur={`${cycleSeconds}s`}
      fill={repeatCount === LOOP ? "remove" : "freeze"}
      keyTimes={keyTimes}
      repeatCount={repeatCount}
      values="1;1;0;0"
    />
  );
}

export default function BuzzLogoAnimation({
  ariaLabel = "Buzz logo animation",
  className = "",
  fullScreen = true,
  loop = false,
  loopRestSeconds = 0,
  reverse = false,
  showBackground = true,
  style,
  textured = true,
  variant = "v8",
}: BuzzLogoAnimationProps) {
  const markRef = useRef<SVGSVGElement>(null);
  const idSuffix = idPart(useId());
  const baseConfig = VARIANTS[variant] ?? VARIANTS.v8;
  const restSeconds = loop ? Math.max(loopRestSeconds, 0) : 0;
  const morphSeconds = Number.parseFloat(baseConfig.duration);
  const cycleSeconds = morphSeconds + restSeconds;
  // Stretch the loop period to morph + rest, packing the morph keyframes into
  // the start of the cycle at native speed. scaleTiming holds the final values
  // for the remainder; the rest-window opacity animation below hides them.
  const config =
    restSeconds > 0
      ? scaleVariant(
          baseConfig,
          `${cycleSeconds}s`,
          morphSeconds / cycleSeconds,
        )
      : baseConfig;
  const animatedConfig = reverse ? reverseVariant(config) : config;
  const repeatCount = loop ? LOOP : "1";
  const maskId = `buzz-logo-cutouts-${idSuffix}`;
  const textureId = `buzz-logo-texture-${idSuffix}`;
  const texture = TEXTURES[config.texture ?? "soft"] ?? TEXTURES.soft;
  const classes = [
    "buzz-logo",
    fullScreen && "buzz-logo--screen",
    !fullScreen && "buzz-logo--compact",
    showBackground && "buzz-logo--background",
    className,
  ]
    .filter(Boolean)
    .join(" ");

  // biome-ignore lint/correctness/useExhaustiveDependencies: restart SMIL when visual props change
  useLayoutEffect(() => {
    const svg = markRef.current;

    if (!svg || typeof svg.setCurrentTime !== "function") {
      return;
    }

    svg.pauseAnimations?.();
    svg.setCurrentTime?.(0);
    svg.querySelectorAll("animate").forEach((animation) => {
      animation.beginElement?.();
    });
    svg.unpauseAnimations?.();
  }, [loop, reverse, restSeconds, textured, variant]);

  return (
    <div className={classes} style={style} role="img" aria-label={ariaLabel}>
      <svg
        ref={markRef}
        className="buzz-logo__mark"
        viewBox="0 0 466 309"
        width="466"
        height="309"
        aria-hidden="true"
      >
        <defs>
          <CutoutMask
            id={maskId}
            config={animatedConfig}
            duration={animatedConfig.duration}
            repeatCount={repeatCount}
          />
          {textured && <TextureFilter id={textureId} texture={texture} />}
        </defs>
        <g filter={textured ? `url(#${textureId})` : undefined}>
          {restSeconds > 0 ? (
            <RestWindowFade
              cycleSeconds={cycleSeconds}
              morphSeconds={morphSeconds}
              repeatCount={repeatCount}
            />
          ) : null}
          <g mask={`url(#${maskId})`}>
            <InkShapes
              config={animatedConfig}
              duration={animatedConfig.duration}
              repeatCount={repeatCount}
            />
          </g>
        </g>
      </svg>
    </div>
  );
}
