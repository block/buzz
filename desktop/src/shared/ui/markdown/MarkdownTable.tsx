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
      <table className="w-max min-w-full border-collapse text-left text-sm">
        {children}
      </table>
    </div>
  );
}
