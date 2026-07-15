/**
 * Buzz bee mark — vendored from squareup/ext-builderbot-ui
 * (public/sites/buzz/bee.svg). Rendered as ink-on-transparent so it inherits
 * the marketing chartreuse background behind it.
 */
export function BeeMark({ className }: { className?: string }) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 466 309"
      className={className}
      aria-hidden="true"
    >
      <defs>
        <mask id="buzz-bee-mask">
          {/* white = show ink, black = cut out */}
          {/* wings */}
          <circle cx="91.7" cy="154.5" r="91.7" fill="white" />
          <circle cx="374.3" cy="154.5" r="91.7" fill="white" />
          {/* body */}
          <rect x="128" y="0" width="210" height="309" rx="34" fill="white" />
          {/* eyes cut out */}
          <circle cx="193.3" cy="84.4" r="27" fill="black" />
          <circle cx="276" cy="84.4" r="27" fill="black" />
          {/* stripe slots cut out */}
          <rect
            x="166.3"
            y="157.2"
            width="136.9"
            height="38.3"
            rx="5"
            fill="black"
          />
          <rect
            x="166.9"
            y="235.1"
            width="136.2"
            height="37.6"
            rx="5"
            fill="black"
          />
        </mask>
      </defs>
      <rect
        x="0"
        y="0"
        width="466"
        height="309"
        fill="#231e1e"
        mask="url(#buzz-bee-mask)"
      />
    </svg>
  );
}
