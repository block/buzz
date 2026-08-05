import * as React from "react";

import { ZorroHat } from "@/shared/ui/zorro-logo/ZorroHat";

export const SlashableHat = React.forwardRef<
  HTMLSpanElement,
  {
    className?: string;
    style?: React.CSSProperties;
    testId?: string;
  }
>(function SlashableHat({ className, style, testId }, ref) {
  return (
    <span
      className={["zorro-slashable-hat", className].filter(Boolean).join(" ")}
      data-testid={testId}
      ref={ref}
      style={style}
    >
      <span className="zorro-slashable-hat__piece zorro-slashable-hat__piece--top">
        <ZorroHat className="h-auto w-full" />
      </span>
      <span className="zorro-slashable-hat__piece zorro-slashable-hat__piece--bottom">
        <ZorroHat className="h-auto w-full" />
      </span>
      <span className="zorro-slashable-hat__slash" />
    </span>
  );
});
