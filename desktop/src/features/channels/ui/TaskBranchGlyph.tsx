import type { SVGProps } from "react";

export function TaskBranchGlyph(props: SVGProps<SVGSVGElement>) {
  return (
    <svg
      role="img"
      aria-label={props["aria-label"] ?? "Branch"}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      {...props}
    >
      <path d="M6 7v10" />
      <path d="M18 4v13" strokeDasharray="1 4" />
      <circle cx="6" cy="4" r="2.5" />
      <circle cx="6" cy="20" r="2.5" />
      <circle cx="18" cy="20" r="2.5" />
    </svg>
  );
}
