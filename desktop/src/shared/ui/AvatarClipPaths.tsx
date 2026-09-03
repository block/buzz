/**
 * One normalized SVG path owns every desktop agent-avatar silhouette. The
 * object-bounding-box coordinate system scales it to the element being clipped,
 * so avatar sizes do not need their own corner definitions.
 */
export const AGENT_AVATAR_SQUIRCLE_CLIP_ID = "agent-avatar-squircle-clip";
export const AGENT_AVATAR_SQUIRCLE_PATH =
  "M .5 0 C .93 0 1 .07 1 .5 C 1 .93 .93 1 .5 1 C .07 1 0 .93 0 .5 C 0 .07 .07 0 .5 0 Z";

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
