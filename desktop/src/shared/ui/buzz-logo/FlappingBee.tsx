import { useId } from "react";

/**
 * The Buzz bee mark with flapping wings. Geometry is identical to the static
 * {@link BuzzMark} (v8 final keyframe) — the same silhouette, rendered in
 * `currentColor` so it tints per-theme — but the two wing lobes carry the
 * `bee-wing` classes so the wing-flap keyframes (ported from the Buzz website)
 * beat them on an infinite loop. It's plain SVG + CSS (no JS/SMIL), so it paints
 * on the very first frame and the flap starts as soon as styles load. Reduced
 * motion falls back to the static silhouette via the CSS media query.
 */
export function FlappingBee({ className }: { className?: string }) {
  const maskId = `flapping-bee-cutouts-${useId().replace(/[^a-zA-Z0-9_-]/g, "")}`;

  return (
    <svg
      aria-hidden="true"
      className={["buzz-mark", "bee-sprite", className]
        .filter(Boolean)
        .join(" ")}
      viewBox="0 0 466 309"
      fill="currentColor"
    >
      <defs>
        <mask
          id={maskId}
          x="-80"
          y="-80"
          width="626"
          height="469"
          maskUnits="userSpaceOnUse"
          maskContentUnits="userSpaceOnUse"
        >
          <rect x="-80" y="-80" width="626" height="469" fill="#fff" />
          <ellipse cx="193.3" cy="84.4" rx="27" ry="27" fill="#000" />
          <ellipse cx="276" cy="84.4" rx="27" ry="27" fill="#000" />
          <rect
            x="166.3"
            y="157.2"
            width="136.9"
            height="38.3"
            rx="5"
            fill="#000"
          />
          <rect
            x="166.9"
            y="235.1"
            width="136.2"
            height="37.6"
            rx="5"
            fill="#000"
          />
        </mask>
      </defs>
      <g mask={`url(#${maskId})`}>
        <circle
          className="bee-wing bee-wing-left"
          cx="91.7"
          cy="154.5"
          r="91.7"
        />
        <circle
          className="bee-wing bee-wing-right"
          cx="374.3"
          cy="154.5"
          r="91.7"
        />
        <rect x="128" y="0" width="210" height="309" rx="34" />
      </g>
    </svg>
  );
}
