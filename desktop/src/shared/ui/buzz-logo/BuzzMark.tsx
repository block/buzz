import { useId } from "react";

/**
 * The finished Buzz mark as a plain static SVG — no SMIL, no scripting, no
 * animation machinery. Geometry matches the final keyframe of the
 * BuzzLogoAnimation morph (v8 variant), rendered in `currentColor`, so it
 * paints complete on the very first frame regardless of animation support.
 */
export function BuzzMark({ className }: { className?: string }) {
  const maskId = `buzz-mark-cutouts-${useId().replace(/[^a-zA-Z0-9_-]/g, "")}`;

  return (
    <svg
      aria-hidden="true"
      className={["buzz-mark", className].filter(Boolean).join(" ")}
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
        <circle cx="91.7" cy="154.5" r="91.7" />
        <circle cx="374.3" cy="154.5" r="91.7" />
        <rect x="128" y="0" width="210" height="309" rx="34" />
      </g>
    </svg>
  );
}
