import { useId } from "react";

/**
 * The Buzz bee mark with flapping wings. Geometry is identical to the static
 * {@link BuzzMark} (v8 final keyframe) — the same silhouette, rendered in
 * `currentColor` so it tints per-theme — with the wing-flap keyframes (ported
 * from the Buzz website) beating the wings on an infinite loop.
 *
 * Unlike the static mark's single `<svg>`, each wing here is its own
 * HTML-level `<svg>` layer and the flap animates those elements' CSS
 * transforms. This is deliberate: WebKit paints SVG *children* on the main
 * thread, so a transform animation on a `<circle>` freezes for as long as boot
 * work (bundle eval, first React render of the app tree) hogs the thread —
 * exactly the window in which the loading gate is on screen. Transforms on
 * HTML-level elements run on the compositor (Core Animation in WKWebView) and
 * keep flapping regardless. The `bee-wing-layer` masks reproduce the slot
 * cutouts over the wings so the layered build stays pixel-identical to the
 * masked single-SVG mark (see animations.css).
 *
 * Everything is plain SVG + CSS (no JS/SMIL), so it paints on the very first
 * frame and the flap starts as soon as styles load. Reduced motion falls back
 * to the static silhouette via the CSS media query.
 */
export function FlappingBee({ className }: { className?: string }) {
  const maskId = `flapping-bee-cutouts-${useId().replace(/[^a-zA-Z0-9_-]/g, "")}`;

  // Wing geometry from the 466x309 mark: circles r=91.7 at (91.7, 154.5) and
  // (374.3, 154.5). Each wing layer is the circle's bounding box, positioned
  // as percentages of the mark: top 62.8/309, size 183.4/466 x 183.4/309.
  const wingLayer =
    "bee-wing-layer absolute top-[20.3236%] h-[59.3528%] w-[39.3562%]";
  const wingSvg = "bee-wing block h-full w-full";

  return (
    <div
      aria-hidden="true"
      className={[
        "buzz-mark",
        "bee-sprite",
        "relative",
        "aspect-[466/309]",
        className,
      ]
        .filter(Boolean)
        .join(" ")}
    >
      <div className={`${wingLayer} bee-wing-layer-left left-0`}>
        <svg
          aria-hidden="true"
          className={`${wingSvg} bee-wing-left`}
          viewBox="0 0 183.4 183.4"
          fill="currentColor"
        >
          <circle cx="91.7" cy="91.7" r="91.7" />
        </svg>
      </div>
      <div className={`${wingLayer} bee-wing-layer-right right-0`}>
        <svg
          aria-hidden="true"
          className={`${wingSvg} bee-wing-right`}
          viewBox="0 0 183.4 183.4"
          fill="currentColor"
        >
          <circle cx="91.7" cy="91.7" r="91.7" />
        </svg>
      </div>
      {/* Body last in DOM order and positioned, so it paints over the wings —
          matching the single-SVG mark where the body rect draws on top. */}
      <svg
        aria-hidden="true"
        className="relative block h-full w-full"
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
        <rect
          x="128"
          y="0"
          width="210"
          height="309"
          rx="34"
          mask={`url(#${maskId})`}
        />
      </svg>
    </div>
  );
}
