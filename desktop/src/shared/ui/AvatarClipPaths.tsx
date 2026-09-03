/**
 * One normalized SVG path owns every desktop agent-avatar silhouette. The
 * object-bounding-box coordinate system scales it to the element being clipped,
 * so avatar sizes do not need their own corner definitions.
 */
type NormalizedPoint = readonly [number, number];
type NormalizedCubic = readonly [
  start: NormalizedPoint,
  firstControl: NormalizedPoint,
  secondControl: NormalizedPoint,
  end: NormalizedPoint,
];

const AGENT_AVATAR_SQUIRCLE_CUBICS: readonly NormalizedCubic[] = [
  [
    [0.5, 0],
    [0.93, 0],
    [1, 0.07],
    [1, 0.5],
  ],
  [
    [1, 0.5],
    [1, 0.93],
    [0.93, 1],
    [0.5, 1],
  ],
  [
    [0.5, 1],
    [0.07, 1],
    [0, 0.93],
    [0, 0.5],
  ],
  [
    [0, 0.5],
    [0, 0.07],
    [0.07, 0],
    [0.5, 0],
  ],
];

export const AGENT_AVATAR_SQUIRCLE_CLIP_ID = "agent-avatar-squircle-clip";
export const AGENT_AVATAR_SQUIRCLE_PATH =
  "M .5 0 C .93 0 1 .07 1 .5 C 1 .93 .93 1 .5 1 C .07 1 0 .93 0 .5 C 0 .07 .07 0 .5 0 Z";

/** Sample the same normalized cubics used by the global avatar clip path. */
export function sampleAgentAvatarSquircle(
  size: number,
  segmentsPerCubic = 32,
): Array<{ x: number; y: number }> {
  const points: Array<{ x: number; y: number }> = [];
  const segments = Math.max(1, Math.floor(segmentsPerCubic));

  for (const [
    start,
    firstControl,
    secondControl,
    end,
  ] of AGENT_AVATAR_SQUIRCLE_CUBICS) {
    for (let index = 0; index < segments; index += 1) {
      const progress = index / segments;
      const remaining = 1 - progress;
      points.push({
        x:
          (remaining ** 3 * start[0] +
            3 * remaining ** 2 * progress * firstControl[0] +
            3 * remaining * progress ** 2 * secondControl[0] +
            progress ** 3 * end[0]) *
          size,
        y:
          (remaining ** 3 * start[1] +
            3 * remaining ** 2 * progress * firstControl[1] +
            3 * remaining * progress ** 2 * secondControl[1] +
            progress ** 3 * end[1]) *
          size,
      });
    }
  }

  return points;
}

export function AvatarClipPaths() {
  return (
    <svg
      aria-hidden="true"
      className="pointer-events-none absolute h-0 w-0 overflow-hidden"
      focusable="false"
    >
      <defs>
        <clipPath
          clipPathUnits="objectBoundingBox"
          id={AGENT_AVATAR_SQUIRCLE_CLIP_ID}
        >
          <path d={AGENT_AVATAR_SQUIRCLE_PATH} />
        </clipPath>
      </defs>
    </svg>
  );
}
