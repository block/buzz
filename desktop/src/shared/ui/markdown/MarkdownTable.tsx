import * as React from "react";

import { useSmoothCorners } from "@/shared/ui/smoothCorners";

export function MarkdownTable({ children }: { children?: React.ReactNode }) {
  const tableBlockRef = React.useRef<HTMLDivElement | null>(null);
  useSmoothCorners(tableBlockRef);

  return (
    <div
      ref={tableBlockRef}
      className="overflow-x-auto rounded-2xl border border-border/70"
      data-table-block=""
    >
      {/* Inherit message wrap-anywhere for long tokens. The cells' minimum
          widths keep short labels readable; many-column tables scroll locally. */}
      <table className="w-full border-collapse text-left text-sm">
        {children}
      </table>
    </div>
  );
}
